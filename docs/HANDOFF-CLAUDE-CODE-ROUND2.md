# Claude Code 执行方案：Round 2 — F2 深修 + C6 Meta + C7 MVU + T2/T3 实跑

> 交接对象：Claude Code（全新会话，无前序上下文）
> 生成时间：2026-06-18
> 前置必读：
> - `docs/HANDOFF-CLAUDE-CODE.md`（Round 1 执行手册，含 harness 背景与接缝约定）
> - `docs/HARNESS-FINDINGS-2026-06-18.md`（发现报告，F1/F2/P3/P4 已修，T2/T3/C1-C8/I1 已覆盖）
> 工作目录：`C:\Users\Predator\ZCodeProject\storyforge`
> 范围：F2 text_fallback 深修 + C6 Meta（4 测试）+ C7 MVU（3 测试）+ T2/T3 实跑

Round 1 已完成并提交（`49fa516`）：F1 model 透传 / F2 prompt 强化（仅治标）/ P3·P4 日志 /
T2·T3·C1-C8·I1 测试文件（多为 `#[ignore]`）/ `cargo test --workspace` 全绿。

本文件是 Round 2 的执行手册。**先读「一、Round 2 背景」理解 Round 1 留了什么坑，
再按「二、任务清单」逐项做。** 所有真实 LLM 测试用 `deepseek-v4-flash` @ opencode.ai
（凭证见末尾「环境」）。

---

## 一、Round 2 背景（必读）

### 1.1 Round 1 留的两个深坑

**坑 A — F2 只治标没治本**：Round 1 给 `CHARACTER_EXTRACTOR_SYSTEM_PROMPT` 加了
"二选一"指令 + ⚠️ 警告，但**没验证真实 LLM 是否真的修好了**。诊断结论是
"deepseek-v4-flash 能力弱、输出自然语言总结、content 里没 JSON、5 层兜底必然全 miss"——
但这个诊断是基于 T1 日志第 34 行的二手信息，**没人抓过模型原始响应的全文**。
到底是"模型其实调了工具但我们的 layer 1 没接住"，还是"模型真没吐任何 JSON"，
**至今未确认**。Round 2 第一步就是补这个诊断，再按结果分支修。

**坑 B — C6/C7 测试基本是空的**：`crates/harness-real-llm/tests/c_command_layer.rs` 里
C1/C2/C5 有真测试，但 **C6 只有 1 个确定性 `c6_health_check`（零 LLM）**，
**C7 一个测试都没有**。Meta 对话链路和 MVU apply 闭环是审计标的重点
（C7 的 `meta_apply_mvu_schema` 被标 dead UI surface，harness 要顶上排 bug），
Round 1 没做完，Round 2 补齐。

### 1.2 测试接缝约定（沿用 Round 1）

- **纯 Rust 库直接调**：`app-meta` 是纯库（不依赖 Tauri runtime），harness 直接
  `use storyforge_app_meta::*` 调 pub API。
- **store 驱动**：`CampaignStore`（`tauri-app/src/lib.rs` 的 `pub mod campaign_store`）
  是 harness 已有的接缝，MVU/Campaign 的存取直接调 store 方法。
- **私有 Tauri 命令调不到**：如 `meta_apply_mvu_schema`（含 backfill loop）是私有命令，
  harness 不能直接调——**测试策略是复刻命令核心逻辑**（用 pub 纯函数 + store 驱动），
  不 pub-ify 命令。这是 Round 1 C1-C8 的既定套路。
- **真实 LLM 测试一律 `#[ignore]`**，`cargo test`（无 `--ignored`）零网络全绿。

---

## 二、任务清单

### 任务 F2：角色识别 text_fallback 深修（🟠 Medium，最先做）

**问题**：Round 1 只做了 prompt 强化（治标）。真实 LLM 跑 `extract_characters` 仍可能
5 层兜底全 miss、降级单角色。详见 `HARNESS-FINDINGS-2026-06-18.md` F2 节。

**关键不确定性**：模型到底输出了什么，没人抓过全文。两种可能，对应两种完全不同的修法：

| 可能 | 现象 | 修法 |
|---|---|---|
| **A** | 模型其实调了 `emit_characters` 工具（`resp.tool_calls` 非空），但 layer 1 没接住（格式歪了） | 加固 layer 1（改一处） |
| **B** | 模型真没调工具也没吐 JSON，纯嘴硬"任务已完成" | 加纠错二次调用（多调一次 LLM） |

**所以：先诊断，再分支。不要一上来就改代码。**

**诊断步骤**（动手前必做）:
1. 读 `crates/app-agent/src/character_extractor.rs` 的
   `parse_character_definitions_from_response`（:135）和 5 层兜底，确认每层在找什么。
   - layer 1：`resp.tool_calls` 里找 `emit_characters`，解析 `arguments` 里的 `characters` 数组
   - layer 2-5：从 `resp.content` 提取 JSON（整体 / ```json 块 / 裸块 / 括号配平）
2. 跑 T1 抓真实响应全文：
   ```bash
   LLM_BASE_URL=... LLM_API_KEY=... LLM_MODEL=deepseek-v4-flash \
     cargo test -p harness-real-llm -- --ignored --nocapture
   ```
   在 `extract_characters`（character_extractor.rs:55 附近，`runtime.run_tool_loop` 之后）
   临时加 `tracing::info!("raw resp content={:?} tool_calls={:?}", resp.content, resp.tool_calls);`
   把模型原始输出打到日志。看 `resp.tool_calls` 是否非空、`resp.content` 里有没有 JSON。
3. 判断 A 还是 B。

**分支 A — 加固 layer 1**（若 tool_calls 非空但 layer 1 漏接）:
可能的具体漏接点（诊断后对号入座）:
- `arguments` 是裸 JSON 数组 `[...]` 而非 `{"characters": [...]}` → layer 1 只取 `args.get("characters")`，
  裸数组会 miss。加一个"arguments 本身就是数组"的分支。
- `arguments` 不是合法 JSON 字符串（模型漏了引号/转义） → 加 lenient 容错或正则抢救。
- tool name 大小写不一致（`Emit_Characters`） → 比较 to_lowercase。
- arguments 里的 key 不是 `characters` 而是别的（`results`/`defs`） → 放宽 key。

**分支 B — 纠错二次调用**（若 tool_calls 空 且 content 无 JSON）:
在 `extract_characters` 里，5 层全 miss 时不直接降级，而是**把 raw content 回喂 LLM 一次**，
强约束重新输出:
- 复用 `make_character_extractor_config()`，换一个纠错 user_msg：
  `"你刚才的回复里没有可解析的角色 JSON。请把你描述的角色，用 JSON 数组重新输出，
   每个对象含 name/persona_prompt/behavior_rules/base_backstory/role_type 字段。只输出 JSON，不要任何解释。"`
- 把 raw content 作为上下文附进 user_msg（让模型复述它"以为"自己输出的内容）。
- 再跑一次 `run_tool_loop`，对第二次响应再走 `parse_character_definitions_from_response`。
- 二次仍失败 → 才降级单角色（保留现有降级路径）。
- **注意**：二次调用会增加 1 次 LLM 请求 + 延迟。记 `tracing::warn!` 标明走了 fallback，
  方便观测。这是有意为之的代价（模型能力不足时的兜底）。

**改哪**：
- 分支 A：`crates/app-agent/src/character_extractor.rs` 的 `parse_character_definitions_from_response` layer 1（:138-153）。
- 分支 B：`crates/app-agent/src/character_extractor.rs` 的 `extract_characters`（:41-77），加二次调用逻辑。

**验收**:
1. 诊断结论（A 还是 B + 证据：raw resp 片段）写进 `HARNESS-FINDINGS-2026-06-18.md` F2 节追加。
2. 真实 LLM 跑 `extract_characters`（seraphina 卡），识别出 ≥1 个角色且**非降级**
   （`extracted=true`，即走了正常解析路径而非 `fallback_from_character`）。
3. 既有 11 个 character_extractor 单测 0 回归 + 新增分支的单测覆盖（A 加 layer1 容错测；B 加二次调用 mock 测）。
4. `cargo test --workspace`（不含 ignored）0 回归。

---

### 任务 C6：Meta 对话链路测试（gap 4a，4 个测试）

**现状**：`c_command_layer.rs` 只有 1 个 `c6_health_check`（零 LLM 确定性）。Meta 对话的
完整链路（start/chat/patches/typed_patch/explain）全缺。

**接缝**：`storyforge_app_meta`（纯库，harness 直接 use）。

#### Meta / Patch pub API 表（已核实）

| 模块 | pub API | 调 LLM？ |
|---|---|---|
| `meta_conversation.rs` | `MetaSession::new/set_character/set_world_info/set_explainer/set_campaign_runtime`；`MetaConversation::new` + `pub async fn chat()`；`GenerationExplainer` trait | chat() 调 |
| `lib.rs` | `PatchStore::{new,propose,pending,accept,dismiss}` + `execute_patch` + `PatchContext` | 纯函数 |
| `typed_patch.rs` | `build_patch_for_issue` / `is_patch_stale` / `apply_to_snapshot` / `build_patch_from_action` + `PreviewInput`/`PreviewInputMut` | 纯函数 |
| `explain.rs` | `explain_generation(provenance) -> GenerationExplanation` | **纯函数，不调 LLM** |
| `health_check.rs` | `check_campaign_health(snapshot) -> Vec<HealthIssue>` + `CampaignHealthSnapshot` | 纯函数 |
| `mvu_import.rs` | `analyze_mvu_card`（async）/ `score_card` / `parse_mvu_translation_from_response` | analyze 调 |

**重要纠正**：Round 1 handoff 的 C6 清单把 `meta_explain_generation` 标了"真实"，
但 app-meta 底层 `explain_generation` 是**纯函数**（从 `Provenance` 拼解释，无 async 无 LlmClient）。
所以 explain 走确定性测试，不是真实 LLM。`meta_propose_campaign_repairs` 在 app-meta
**无独立 pub 入口**（是 Tauri 命令层用 `MetaSession.chat` + 特定 prompt 组装的）——
C6 用 `chat()` 驱动即可覆盖底座，repairs 不单独测。

#### 4 个测试（在 `c_command_layer.rs` 的 C6 节追加）

1. **`c6_meta_chat_real_llm`**（真实 LLM，`#[ignore]`）
   - `MetaSession::new()` + `set_character(...)` + `MetaConversation::new()` + `chat()`。
   - 问个简单问题（如"这张卡有哪些角色？请简述"），断言 `MetaConversation` 有 turn 累积、
     返回的 `MetaTurn` 含 content 非空。
   - 参考 `meta_conversation.rs` 的 `chat()` 签名与返回类型。

2. **`c6_patch_store_lifecycle`**（确定性）
   - `PatchStore::new()` → `propose(...)` → `pending()` 含该项 → `accept(&id)` → `pending()` 空。
   - 再 propose 一个 → `dismiss(&id)` → `pending()` 空。
   - 零 LLM，纯状态机。

3. **`c6_typed_patch_preview_apply`**（确定性）
   - 构造一个 `HealthIssue`（如缺角色定义）+ `PreviewInput`（喂 Character/Campaign 快照）→
     `build_patch_for_issue` → 断言得到 `TypedPatch` → `is_patch_stale` 为 false →
     `apply_to_snapshot` 应用 → 断言快照字段已变。
   - 参考 `typed_patch.rs` 的测试（该文件内已有单测可借模式）。

4. **`c6_explain_generation`**（确定性）
   - 构造一个 `Provenance`（子 agent 成文溯源）→ `explain_generation(&provenance)` →
     断言 `GenerationExplanation` 含非空字段。
   - 纯函数，验证 explain 闭环不崩。

**验收**：4 个测试全绿（3 确定性默认跑 + 1 真实 `#[ignore]`）；`cargo test --workspace` 0 回归。

---

### 任务 C7：MVU 流测试（gap 4b/4c/4d，3 个测试）

**现状**：C7 完全没有测试。审计标的 `meta_apply_mvu_schema` 为 dead UI surface，
harness 要顶上排 bug。

**接缝**：
- `analyze_mvu_card`（app-meta pub，真实 LLM）
- `compute_apply_preview` + `apply_schema_to_definition`（app-meta pub，**纯函数，per-definition，无 store lock**）
- `CampaignStore`（tauri-app pub）的 mvu 存取方法（`get_mvu`/list 等，grep `fn .*mvu` in `campaign_store` 确认签名）

#### MVU pub API 表（已核实）

| 模块 | pub API | 调 LLM？ |
|---|---|---|
| `mvu_import.rs` | `analyze_mvu_card(runtime, character, cancel)` → `MvuTranslation` | 调（失败降级 `pure_data_fallback`） |
| `mvu_import.rs` | `score_card(character)` → `CardComplexityReport` | 纯函数 |
| `mvu_apply.rs` | `compute_apply_preview(current, mvu, def_id, name, src_id)` → `MvuApplyPreview` | 纯函数 |
| `mvu_apply.rs` | `apply_schema_to_definition(&mut def, merged_schema)` | 纯函数 |

#### ⚠️ backfill loop 位置（C7 测试的核心价值）

MVU apply 的 backfill loop（把合并 schema 批量写回所有匹配 instance、补新变量）
在 **`tauri-app/src/lib.rs:3822-3845`**，`meta_apply_mvu_schema` 私有命令体内：

```rust
// Best-effort：对已存在的 instances 补齐新变量
for instance in store.list_instances(&card.id) {
    if instance.definition_id.as_ref() != Some(&def_id) { continue; }
    let mut updated = instance.clone();
    let new_fields: Vec<_> = preview.added_fields.iter()
        .filter(|f| !updated.variables.iter().any(|v| v.key == f.key))
        .collect();
    if new_fields.is_empty() { continue; }
    for field in new_fields {
        updated.variables.push(VariableValue::new(field.key.clone(), field.default.clone(), 0));
    }
    store.update_instance(updated);
}
```

这段是私有的，harness 调不到 → **4d 测试策略：复刻这段逻辑**（用 pub 纯函数 + store 驱动），
把 backfill 行为钉住。将来命令改了、backfill 逻辑漂移，测试能发现。这是 C7 测试真正的价值。

#### 3 个测试（在 `c_command_layer.rs` 的 C7 节新建）

1. **`c7_mvu_analyze_real_llm`**（gap 4b，真实 LLM，`#[ignore]`）
   - 导入 seraphina 卡 → `analyze_mvu_card(&runtime, &character, cancel)` →
     断言 `MvuTranslation` 非降级（`routing` 非 Native 或 `variable_schema`/`ui_bindings` 非空，
     视卡内容而定；至少不 panic、不返回 `pure_data_fallback` 的空壳——seraphina 卡有 JS，不应短路）。
   - 参考 `mvu_import.rs` 末尾的 `test_analyze_mvu_card_with_mock`（用真实 LLM 换掉 mock）。

2. **`c7_mvu_translation_crud`**（gap 4c，确定性）
   - 构造一个 `MvuTranslation`（可用 `MvuTranslation::pure_data_fallback(...)` 或手搓）→
     存进 `CampaignStore`（grep 确认存取方法名，可能是 `save_mvu`/`update_mvu`）→
     `store.get_mvu(&id)` 取回 → 断言字段一致 → list 全量含此项。
   - 零 LLM，纯 store CRUD。

3. **`c7_mvu_apply_backfill`**（gap 4d，确定性）
   - 建卡（`make_minimal_card` + 一个 definition，schema 含 `hp`）→
     `save_card` + `create_campaign`（产生 instance，`definition_id` 指向该 def）→
     构造一个 MVU schema（含 `hp` 覆盖 + 新增 `mp`）→
     `compute_apply_preview(&def.variable_schema, &mvu_schema, ...)` → 断言 `added_fields` 含 `mp`、
     `has_changes` 为 true →
     `apply_schema_to_definition(&mut def, preview.merged_schema)` → `update_card` →
     **复刻 backfill loop**（照搬 lib.rs:3822-3845 逻辑）：遍历 `list_instances`，按 `definition_id`
     过滤，给每个 instance 补 `added_fields` 对应 `VariableValue` → `update_instance` →
     断言 instance.variables 含 `mp` 且 default 值正确、不含重复 key。
   - 这是 C7 最有价值的测试：把私有 backfill 行为钉死。

**验收**：3 个测试全绿（2 确定性 + 1 真实 `#[ignore]`）；`cargo test --workspace` 0 回归。

---

### 任务 T2/T3：实跑真实 LLM（验证 Round 1 写的测试）

**现状**：Round 1 写了 `tests/t2_multi_turn.rs`（3 轮 append）和 `tests/t3_regenerate.rs`
（3 种 reroll），都 `#[ignore]`，但**从未实跑过**——只确认了编译通过 + 确定性测试绿。

**任务**：跑一遍真实 LLM，确认通过。若有失败：
- **优先修测试**（断言写错、接缝用错、fixture 路径），不修业务。
- 若失败确属业务 bug（如多轮 turn 不递增、regenerate 变体树没长），记进 findings，
  报告但不擅自改业务（超 Round 2 范围）。

**怎么跑**:
```bash
LLM_BASE_URL='...' LLM_API_KEY='...' LLM_MODEL=deepseek-v4-flash \
  cargo test -p harness-real-llm -- --ignored --nocapture
```
（会连带跑 T1 + I1 + C6/C7 的真实 LLM 项；可单独 `--test t2_multi_turn` / `--test t3_regenerate` 收窄。）

**T2 断言要点**（参考 Round 1 handoff T2 节）:
- turn 递增（`fill_campaign_context` 每次 `list_summaries().len()+1`）
- 第 2/3 轮 `recent_messages` 含前轮成文
- `round_summaries` 累积
- postprocess 写回的知识在第 2 轮 `campaign_runtime.knowledge` 可见

**T3 断言要点**:
- `regenerate_all` / 仅 director / 仅 editor 三种 reroll，`conv.find_node(node_id).variants.len()` 递增

**验收**：T2/T3 真实 LLM 跑通（或失败原因明确记录）。结果写进
`HARNESS-FINDINGS-2026-06-18.md` 追加「T2/T3 实跑结果」节。

---

## 三、执行顺序建议

1. **F2 诊断**（抓 T1 raw response，判 A/B）——最先，因为 C7 的 `c7_mvu_analyze_real_llm`
   和 T2/T3 实跑都要跑真实 LLM，F2 诊断时顺手把真实 LLM 环境跑通。
2. **F2 修 + 验收**（按 A 或 B 分支）。
3. **C6**（4 测试，3 确定性先写，1 真实 LLM 留到实跑批）。
4. **C7**（3 测试，2 确定性先写，1 真实 LLM 留到实跑批）。
5. **T2/T3 实跑**（连带 C6/C7 真实 LLM 项一起跑 `--ignored`）。

每做完一个任务：跑相关测试 + 更新 `HARNESS-FINDINGS-2026-06-18.md` 对应节状态。

---

## 四、环境

真实 LLM 凭证（设环境变量跑 `--ignored` 测试）:
```
LLM_BASE_URL=https://opencode.ai/zen/go/v1/chat/completions
LLM_API_KEY=<your-key-here>
LLM_MODEL=deepseek-v4-flash
```

> API key 不写进本文件（防进 git 历史）。运行真实 LLM 测试前，由用户单独提供 key 并设为环境变量。
> fixture：仓库根 `test-card-seraphina.png`（551KB 真实 ST 卡，测试里用 `find_fixture()` 向上查找）。

---

## 五、红线（不要做）

- 不要改前端（Vue）——本次纯后端 + 测试。
- 不要改既有 `PLAN-*.md` 计划正文（除状态行）。
- 不要为图快把 `meta_apply_mvu_schema` 等私有命令 pub-ify——C7 测试用纯函数 + store 复刻，
  不改命令可见性（pub-ify 会污染 tauri-app 的 API 表面）。
- **F2 不要跳过诊断直接改**——A 和 B 修法完全不同，瞎改必返工。先抓 raw response 判清楚。
- 真实 LLM 测试必须 `#[ignore]`，`cargo test`（无 `--ignored`）必须零网络全绿。
- 不要 commit（用户没要求）。

---

## 六、给 Claude Code 的提示词

```
请阅读 docs/HANDOFF-CLAUDE-CODE-ROUND2.md（本文件）和 docs/HARNESS-FINDINGS-2026-06-18.md，
然后按本文件「三、执行顺序建议」逐项执行所有任务（F2 深修、C6、C7、T2/T3 实跑，全部）。

工作目录：C:\Users\Predator\ZCodeProject\storyforge

关键约束：
- F2 最先做，但必须先诊断（抓 T1 raw response 判 A/B 分支）再改代码，不要跳过诊断。
- C7 的 c7_mvu_apply_backfill 测试要复刻 tauri-app/src/lib.rs:3822-3845 的 backfill loop
  （用 pub 纯函数 + CampaignStore 驱动），不要 pub-ify 私有命令。
- 真实 LLM 测试一律 #[ignore]，cargo test（不带 --ignored）必须零网络全绿。
- 每个任务做完跑相关测试 + 更新 HARNESS-FINDINGS-2026-06-18.md 对应节状态。
- 不改前端、不改 PLAN 正文、不 pub-ify 私有命令、不 commit。
- 真实 LLM 凭证用环境变量（见本文件「四、环境」），不要写进任何文件。

先从 F2 诊断开始。动手前先读 crates/app-agent/src/character_extractor.rs 的
parse_character_definitions_from_response（5 层兜底）和 HARNESS-FINDINGS 的 F2 节，
确认理解根因后再跑 T1 抓 raw response。
```

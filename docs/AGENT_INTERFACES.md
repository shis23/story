# Agent 接口与提示词索引

> 最后更新：2026-06-16
> 目的：**集中记录每个 Agent 的 system prompt、上下文拼装、输出解析的代码位置与修改方法**。
> 改 prompt 时只看这一个文件，不用全仓库翻。
>
> 设计原则：**所有 prompt 都是纯字符串常量/模板函数，没有隐藏拼接**。改一处，效果立即可见。

---

## 0. 快速导航（改某个 Agent 该改哪个文件）

| 我想改… | 改这个文件 | 改哪个符号 |
|---------|-----------|-----------|
| 导演的系统提示词（角色定位/任务/输出格式） | `crates/app-pipeline/src/lib.rs` | `DIRECTOR_SYSTEM_PROMPT` 常量（行 53，走 assemble_system_prompt 模块化） |
| 导演的「消息结构」（§22 三段：system 蓝灯 + history + tail 意图/任务） | `crates/app-pipeline/src/lib.rs` | `build_director_system_extra()`（蓝灯进 system，行 1111）+ `build_director_tail()`（意图/任务进 tail，行 1146） |
| 编剧的系统提示词 | `crates/app-pipeline/src/lib.rs` | `EDITOR_SYSTEM_PROMPT` 常量（行 71，走 assemble_system_prompt 模块化） |
| 子 Agent 的系统提示词模板 | `crates/app-pipeline/src/lib.rs` | `SUBAGENT_SYSTEM_PROMPT_TEMPLATE` 常量（行 79，含 `{name}` 占位符） |
| 子 Agent 的「专属上下文包」拼装（角色设定/场景/世界书/最近对话） | `crates/app-pipeline/src/lib.rs` 或 `crates/app-agent/src/runtime.rs` | `format_subagent_context()`（两处实现，见 §3.2） |
| 导演/编剧/子 Agent 的工具集（能调什么工具） | `crates/app-agent/src/tools.rs` | `register_director_tools()` / `register_subagent_tools()` / `register_editor_tools()` |
| 导演的工具调用最大轮次 / 默认模型 | `crates/app-pipeline/src/lib.rs` | `make_director_config()`（行 1137）/ `make_editor_config()`（行 1158） |
| 导演 Plan 的解析（JSON→Plan 结构） | `crates/app-pipeline/src/lib.rs` | `parse_plan_from_response()`（行 1187）+ 5 层兜底 |
| **角色识别 Agent 的系统提示词 / config / 工具 / 输出解析** | `crates/app-agent/src/prompts/character_extractor.rs` + `crates/app-agent/src/character_extractor.rs` | `CHARACTER_EXTRACTOR_SYSTEM_PROMPT` / `make_character_extractor_config()` / `parse_character_definitions_from_response()`（5 层兜底，见 §6.2）|
| 重 roll 时注入用户反馈（hint） | `crates/app-agent/src/runtime.rs` | `inject_hint_into_subagent()` / `inject_hint_into_editor()` |
| 提示词「模块」（视角/文风/CoT/约束，三层预设体系） | `crates/domain/src/prompt_module.rs` | `assemble_system_prompt()` + `builtins::preset_modules()` |
| **Meta Agent 的系统提示词 / 多轮对话框架 / 诊断工具 / Patch** | `crates/app-meta/src/prompts/meta_agent.rs` + `crates/app-meta/src/meta_conversation.rs` + `crates/app-meta/src/lib.rs` | `META_AGENT_SYSTEM_PROMPT`（行 28）/ `chat()`（行 146）/ `inspect_world_info` / `inspect_character` / `PatchStore`（见 §13） |
| **Meta Agent 的 MVU 五合一分析 / ST 预设分类** | `crates/app-meta/src/mvu_import.rs` + `crates/app-meta/src/prompts/mvu_analyzer.rs` | `MVU_ANALYZER_SYSTEM_PROMPT`（行 27）/ `analyze_mvu_card()`（行 41）/ `classify_st_preset_with_llm()`（行 529）（见 §8.3） |
| **角色/全局变量表的字段定义** | `crates/domain/src/variables.rs` | `default_character_variables()` 工厂 + `VariableField` 结构（见 §7） |
| **变量的注入提示词模板** | `crates/domain/src/variables.rs` | `render_variables_for_injection()`（见 §7.5） |
| **任务（伏笔/计划）的数据结构** | `crates/domain/src/story_task.rs` | `StoryTask` + `TaskTrigger`（见 §9） |
| **任务注入导演提示词的位置** | `crates/app-pipeline/src/lib.rs` | `build_director_tail()` 末尾追加（见 §9.4） |
| **剧情总结 Agent 的系统提示词 / config / 用户消息** | `crates/app-agent/src/prompts/summarizer.rs` + `crates/app-agent/src/summarizer.rs` | `SUMMARIZER_SYSTEM_PROMPT` / `make_summarizer_config()` / `run_summarizer()`（见 §6.3） |
| **后处理 Agent 的系统提示词 / config / 用户消息 / 输出解析** | `crates/app-agent/src/prompts/postprocess.rs` + `crates/app-agent/src/postprocess.rs` | `POSTPROCESS_SYSTEM_PROMPT` / `make_postprocess_config()` / `run_postprocess()` / `parse_postprocess_from_response()`（5 层兜底，见 §6.4） |
| **后处理并行编排（总结 + 后处理并发）** | `crates/app-agent/src/pipeline_postprocess.rs` | `run_postprocess_pipeline()` → tokio::join!（见 §6.4） |
| **后处理结果持久化（知识/变量/任务/摘要落盘）** | `crates/tauri-app/src/lib.rs` | `persist_postprocess_outcome()` + `fill_campaign_context()` |
| **task/knowledge/summary 的 Tauri 命令** | `crates/tauri-app/src/lib.rs` | `list_character_knowledge` / `list_tasks` / `create_task` / `complete_task` / `abandon_task` / `list_round_summaries`（见 §9.5） |
| **cache 友好消息布局（§22 已落地，三段分离）** | `crates/domain/src/message_layout.rs` + `crates/app-agent/src/runtime.rs` | `MessageLayout` + `run_tool_loop_with_layout()`（行 313，消费 layout）（见 §10） |

---

## 1. 三层 prompt 架构（先理解这个，再看具体 Agent）

每个 Agent 最终发给 LLM 的 system prompt，由「角色定位 + 模块 + 上下文」组装：

```
最终 system_prompt = assemble_system_prompt(role, role_directive, profile, modules, tool_directives)
                     └─── 来自 crates/domain/src/prompt_module.rs
```

```
┌──────────────────────────────────────────────┐
│ Layer A：role_directive（角色定位，固定）       │  ← 硬编码常量（§2/§3/§4）
│   「你是写作导演…」「你是编剧…」              │
├──────────────────────────────────────────────┤
│ Layer B：profile 选中的模块（可配置）           │  ← crates/domain/src/prompt_module.rs
│   视角/文风/CoT/质量约束/输出规范/基调         │     assemble_system_prompt() 按 category 顺序拼
├──────────────────────────────────────────────┤
│ Layer C：自定义 override（用户直改）           │  ← PromptProfile.overrides
├──────────────────────────────────────────────┤
│ Layer D：tool_directives（工具说明，自动生成）  │  ← 从 ToolRegistry.tool_specs() 生成
└──────────────────────────────────────────────┘
```

> ✅ **当前状态（2026-06-16 核实）**：导演 / 编剧**已接入**模块系统。`make_director_config`（`lib.rs:1137`）和 `make_editor_config`（`lib.rs:1158`）都调 `assemble_system_prompt(role, role_directive, profile, modules, "")`，所有调用点（start_writing / regenerate 各路径）都传 `ctx.profile` + `ctx.modules`。
>
> **各 Agent 模块化状态**：
> | Agent | 模块化？ | 说明 |
> |-------|---------|------|
> | 导演 | ✅ | 走 assemble_system_prompt，profile 选中的模块拼进 system |
> | 编剧 | ✅ | 同上 |
> | 子 Agent | ❌ | `spawn_subagents`（`runtime.rs:350`）用 `format!` 直接拼 `base_prompt + 角色名 + context_package + brief`，**不走** assemble_system_prompt。子 Agent 的模块化是缺口 |
> | 角色识别 / 总结 / 后处理 / Meta | ❌ | 各 crate 独立 prompt 常量，不走三层预设体系 |
>
> **改 system prompt 的两条路**：
> - 改**所有 Agent 共有的角色定位** → 改 `*_SYSTEM_PROMPT` 常量（role_directive，模块系统在其后拼接）
> - 改**单个 Agent 的文风/视角/约束** → 走模块编辑（`prompt_module.rs::preset_modules()` / 高玩模式 UI 加模块绑 Profile），不碰常量

---

## 2. 导演 Agent（Director）

### 2.1 系统提示词

**位置**：`crates/app-pipeline/src/lib.rs:53` 的 `DIRECTOR_SYSTEM_PROMPT` 常量

**内容骨架**：
```
你是写作导演。用户给你写作意图，你要：
1. 调用 search_world_info / get_character 了解可用素材
2. 判断本场戏：核心冲突是什么？引入哪些角色？
3. 决定出场角色，为每个角色分配任务
4. 为每个角色构造专属上下文包
5. 输出结构化 Plan
不要自己写正文。
【输出格式】调用 emit_plan 工具，或直接输出 JSON：
{"scene_brief": "...", "subagent_tasks": [{"character_id": "...", "brief": "..."}]}
```

**怎么改**：直接编辑常量字符串。注意输出格式段（JSON 示例）改了要同步改 `parse_plan_from_response`（§2.4）。

### 2.2 消息结构（§22 cache 友好三段布局，已落地）

导演的 LLM 调用走 `run_tool_loop_with_layout`（`app-agent/runtime.rs:313`），消息分三段，由 `MessageLayout` 组装（`domain/message_layout.rs`）：

**[1] system 段（稳定，整个会话不变，cache 全命中）**：
- `make_director_config`（`lib.rs:1181`）构造：`assemble_system_prompt(role_directive + 模块)` + **蓝灯世界设定**（`build_director_system_extra`，`lib.rs:1111`，蓝灯条目按 depth 升序，depth 小的排后=更重视）
- 蓝灯移进 system 是 §22 的关键——它整个会话稳定，进 system 段让前缀稳定、cache 命中

**[2] history 段（稳定前缀，逐轮 append）**：
- `conv_store.recent_messages_as_chat(conv_id, 20, before_node_id)`（`lib.rs` 调用处）返回真正的 `[user, assistant, ...]` 消息列表（role 映射，content 无前缀）
- regenerate 时 `before_node_id = Some(node_id)` 排除重 roll 目标及之后

**[3] tail 段（易变，每轮新建，用完即弃）** — `build_director_tail`（`lib.rs:1146`）：
1. `用户的写作意图：{intent}\n\n可用角色：{角色名}`（角色名来自 `ctx.characters`）
2. `render_tasks_for_injection(pending_tasks, turn, story_clock)` — 任务/伏笔（确定性查表零 LLM，只注入 Pending/Active 且触发满足的）
3. 尾句 `请分析意图并输出 Plan。`

> ✅ **§22 已落地（2026-06-16）**：蓝灯进 system、对话历史进独立 history 段、意图/任务进 tail。`prefix_fingerprint` 测试验证「两轮不同 intent/turn 但相同蓝灯 → system 指纹一致」（cache 命中）。
>
> ⚠️ **变量注入尚未接入**（§23.5 缺口）：`render_variables_for_injection()` 已就绪但未被 `build_director_tail` 调用。当前变量不进任何 prompt。待角色体系接通后补（见 `docs/PLAN-CHARACTER-UNIFICATION.md` 阶段 3）。

**怎么改**：
- 改**角色定位/输出格式** → `DIRECTOR_SYSTEM_PROMPT` 常量
- 改**蓝灯注入** → `build_director_system_extra`
- 改**易变内容**（意图/任务/变量）→ `build_director_tail`
- 加新的易变内容 → push 进 `build_director_tail` 的 VolatileTail（别进 system/history）

### 2.3 工具集

**位置**：`crates/app-agent/src/tools.rs:102` 的 `register_director_tools(&mut ToolRegistry)`

注册的工具：
- `search_world_info` — 关键词检索世界书
- `get_character` — 查角色卡详情
- `search_vectors` — 向量检索（记忆 + 绿灯世界书）
- `emit_plan` — 输出 Plan（工具调用形式）

**怎么改**：在函数里 `registry.register(...)` 新增工具，或在 `crates/app-agent/src/tools.rs` 顶部加工具实现函数。

### 2.4 Plan 解析（导演输出 → Plan 结构）

**位置**：`crates/app-pipeline/src/lib.rs:979` 的 `parse_plan_from_response()` 函数

**5 层兜底**（从严到松）：
1. `emit_plan` 工具调用结果
2. 整个 content 是 JSON
3. ` ```json ` 代码块
4. 裸代码块
5. **手写括号配平**（`match_braces`，从每个 `{` 开始计数深度到配平的 `}`）—— 不用正则，中文 UTF-8 安全

**怎么改**：改 Plan 的 JSON schema（比如加字段）→ 改 `parse_plan_json()` + `parse_context_package()`（`lib.rs:1110` / `1172`）。

### 2.5 运行配置

**位置**：`crates/app-pipeline/src/lib.rs:1181` 的 `make_director_config()`

- `max_tool_rounds: 15`（导演最多调 15 轮工具，超过报错）
- `model: "deepseek-chat"`（默认模型，运行时从活跃连接覆盖）
- 接收 `system_extra`（蓝灯世界设定）参数，拼进 system_prompt 末尾（§22）

**怎么改**：改这里的常量，或接入 AgentBinding 让用户配。

---

## 3. 子 Agent（Subagent，N 个并行）

### 3.1 消息结构（§22 cache 友好布局，已落地）

子 Agent 走 `run_tool_loop_with_layout`（`runtime.rs:313`），消息分两段（无 history 段——子 Agent 是对话隔离容器，不看全局对话）：

**[1] system 段（整个 campaign 稳定，cache 全命中）** — 在 `spawn_subagents`（`runtime.rs:473`）拼装：
```
{SUBAGENT_SYSTEM_PROMPT_TEMPLATE}
\n\n你是角色 {character_id}。
\n\n{format_context_stable(task.context_package)}   ← 角色设定(persona) + 常驻世界设定
```
- `format_context_stable`（`runtime.rs:592`）：`character_brief`（persona）+ `constant_lore`（蓝灯）
- persona 进 system 是 §22 的最大 cache 收益点——同一角色跨场戏 system 段 byte 一致

**[2] tail 段（每场戏变，用完即弃）**：
```
{task.context_package.task}   ← 导演分配的具体任务
\n\n{format_context_volatile(task.context_package)}   ← 场景/相关设定/最近对话
\n\n{task.brief}
```
- `format_context_volatile`（`runtime.rs:614`）：`scene_brief` + `relevant_lore`（绿灯）+ `recent_window`

> ✅ **§22 已落地（2026-06-16）**：persona + 常驻世界设定进 system（稳定），场景/相关设定/任务进 tail（易变）。子 Agent system 跨场戏稳定，cache 命中率最高。
>
> ⚠️ **子 Agent 不走模块系统**：与导演/编剧不同，子 Agent system 在 `spawn_subagents` 用 `format!` 拼装，不调 `assemble_system_prompt`。文风/约束模块化仍是缺口。
>
> ⚠️ **信息隔离未落地**（§16 缺口）：当前子 Agent tail 的「最近对话」是共享 `recent_window`，**未按角色过滤**（角色不该看到不在场时发生的事）。待角色体系接通后补（见 `docs/PLAN-CHARACTER-UNIFICATION.md` 阶段 4）。

**怎么改**：改模板字符串改 `SUBAGENT_SYSTEM_PROMPT_TEMPLATE`（`lib.rs:79`）；改稳定/易变拆分改 `format_context_stable`/`format_context_volatile`（`runtime.rs:592/614`）。

### 3.2 专属上下文包（ContextPackage）

⚠️ **关键设计**：子 Agent 是**上下文隔离容器**。它只能看到导演构造的 `ContextPackage`，看不到其他子 Agent 的产出、看不到全局对话树。

**ContextPackage 结构定义**：`crates/domain/src/agent.rs`（含 character_brief/scene_brief/constant_lore/relevant_lore/recent_window/task 字段）

**格式化函数（拆成稳定/易变两份，§22）**：
- `app-agent/runtime.rs:592` `format_context_stable()` — character_brief + constant_lore（进 system）
- `app-agent/runtime.rs:614` `format_context_volatile()` — scene_brief + relevant_lore + recent_window（进 tail）
- `app-pipeline/lib.rs:1431` `format_subagent_context_stable()` / `:1449` `format_subagent_context_volatile()` — 重 roll 单子 Agent 路径用的同步拷贝（同算法独立实现，避免跨 crate 耦合）

**怎么改**：
- 要加「该角色已知信息」字段 → 改 `ContextPackage` struct + 两个 `format_*` 函数
- 要改注入顺序/格式 → 改两个 `format_*` 函数（**两处都要改**，否则行为不一致）

### 3.3 工具集

**位置**：`crates/app-agent/src/tools.rs:314` 的 `register_subagent_tools(&mut ToolRegistry)`

子 Agent 工具更少（限制权限）：只读自己的上下文 + 向量检索。

**怎么改**：同 §2.3。

### 3.4 重 roll 时注入 hint

**位置**：重 roll 单子 Agent 时，hint 以 `【导演反馈】{hint}` 标记追加到 tail 末尾（`app-pipeline/lib.rs` 路径 C 的 spawn 闭包内，用 `SUBAGENT_HINT_MARKER`）。

把用户反馈（「上次演得太僵硬」）告知该角色上次哪里演得不对。

**怎么改**：改 `SUBAGENT_HINT_MARKER` 常量（`runtime.rs:616`）或注入逻辑。

---

## 4. 编剧 Agent（Editor）

### 4.1 系统提示词

**位置**：`crates/app-pipeline/src/lib.rs:71` 的 `EDITOR_SYSTEM_PROMPT` 常量

```
你是编剧。收集所有子 Agent 的表演，合并成连贯成文：
1. 节奏把控、视角切换、过渡衔接
2. 输出最终成文（Markdown）
3. 标注哪些子表演被你裁剪/改动了
直接输出成文，不需要调用工具。
```

**怎么改**：直接改常量。

### 4.2 消息结构（§22 cache 友好布局，已落地）

编剧走 `run_tool_loop_with_layout`，消息分三段：

**[1] system 段（稳定）**：`make_editor_config`（`lib.rs:1231`）→ `assemble_system_prompt(EDITOR_SYSTEM_PROMPT + 模块)`。编剧 system 不含蓝灯（编剧不需要世界设定）。

**[2] history 段**：`conv_store.recent_messages_as_chat(conv_id, 20, before_node_id)`（regenerate 时排除目标节点）

**[3] tail 段** — `build_editor_tail`（`lib.rs:1211`）：
1. `场景：{scene_brief}\n\n子 Agent 表演：\n\n{performances_text}\n\n请合并成连贯成文。`
2. 可选 hint：`【上次问题】{hint}`（`EDITOR_HINT_MARKER`）

> ✅ §22 已落地：编剧 system（role_directive + 模块）+ history（独立消息段）+ tail（场景/子产出/hint）。

**怎么改**：改 tail 内容改 `build_editor_tail`；改 system 改 `EDITOR_SYSTEM_PROMPT` 或模块。

### 4.3 工具集

**位置**：`crates/app-agent/src/tools.rs` 的 `register_editor_tools(&mut ToolRegistry)`

**怎么改**：同 §2.3。

### 4.4 运行配置

**位置**：`crates/app-pipeline/src/lib.rs:1231` 的 `make_editor_config()`

- `max_tool_rounds: 5`（编剧很少调工具，5 轮够）
- `model: "deepseek-chat"`

### 4.5 重 roll 时注入 hint

**位置**：`crates/app-agent/src/runtime.rs:473` 的 `inject_hint_into_editor(user_message, hint)`

标记是 `【上次问题】`（常量 `EDITOR_HINT_MARKER`，`runtime.rs:457`）。

---

## 5. 三层预设体系（模块系统）

> 当前是「数据结构 + 预置模块」就绪，**尚未接入流水线**。接入点见 §5.4。

### 5.1 数据结构

**位置**：`crates/domain/src/prompt_module.rs`

| 结构 | 行号 | 含义 |
|------|------|------|
| `PromptModule` | 44 | 最小单位（视角/文风/CoT…），含 content/applicable_roles/tags |
| `PromptProfile` | 70 | 模块组合（每 Agent 选哪些模块），可保存命名 |
| `AgentBinding` | 123 | 运行时绑定（Agent → Profile + 连接） |
| `ModuleCategory` | 10 | 6 大分类组 |
| `Exclusivity` | 27 | Single 单选互斥 / Multiple 多选叠加 |

### 5.2 组装函数

**位置**：`crates/domain/src/prompt_module.rs:134` 的 `assemble_system_prompt(role, role_directive, profile, modules, tool_directives)`

**拼接顺序**：role_directive → 按 category 顺序（Perspective→Cot→Style→Tone→Quality→Output）插入模块 → override 文本 → tool_directives。段间用 `\n\n---\n\n` 分隔。

### 5.3 预置模块

**位置**：`crates/domain/src/prompt_module.rs:179` 的 `builtins` 模块

- `preset_modules()`（206 行）：5 个内置模块（第三人称/白描/杀八股/通用CoT/字数控制）
- `default_profile()`（267 行）：默认 Profile，导演绑 CoT，编剧绑视角+文风+约束+输出，子 Agent 绑输出

**怎么改/加模块**：在 `preset_modules()` 里加 `PromptModule { ... }`，在 `default_profile()` 里把它绑到对应 Agent。

### 5.4 接入流水线（✅ 已接入导演/编剧，子 Agent 缺口）

**接入点**：`make_director_config()` / `make_editor_config()`（`lib.rs:1137/1158`）

✅ 这两个函数都已调 `assemble_system_prompt`：
```rust
let system_prompt = storyforge_domain::prompt_module::assemble_system_prompt(
    &AgentRole::Director,
    DIRECTOR_SYSTEM_PROMPT,          // role_directive
    profile,                          // PromptProfile（来自 WritingContext.profile）
    modules,                          // 所有可用模块（来自 WritingContext.modules）
    "",                               // tool_directives（当前传空，工具说明另走 LLM 工具协议）
);
```

所有调用点（`start_writing` lib.rs:228/358、`regenerate` 各路径 lib.rs:619/824/973）都传 `ctx.profile.as_ref()` + `&ctx.modules`。`WritingContext.profile` / `.modules` 由 `tauri-app` 的 `fill_profile_context()`（tauri-app/src/lib.rs）从活跃 Profile + ModuleStore 填充。

❌ **子 Agent 缺口**：`spawn_subagents`（runtime.rs:350）不走 assemble_system_prompt，见 §3.1。

> **tool_directives 当前传空串**：工具说明不走 prompt 文本，而是走 LLM 的 function calling 协议（OpenAI tools 字段）或 XML/JSON 降级协议（text_tools.rs）。`assemble_system_prompt` 的 `tool_directives` 参数预留给「不支持工具协议的模型用文本说明工具」的场景，目前未启用。

---

## 6. 新增 Agent 的接口规范（角色识别 / 剧情总结 / 后处理）

> 这三个 Agent 是本轮设计新增（见 TECHNICAL_DESIGN.md §16-§18），**实现时遵循以下接口约定**，保持和现有 Agent 一致的可改性。

### 6.1 通用约定

每个新 Agent 都要有：
1. **一个 `*_SYSTEM_PROMPT` 常量**（放在它的 crate 里，如 `app-pipeline/src/lib.rs` 或新建 `app-postprocess/`）
2. **一个 `make_*_config()` 函数**（构造 AgentConfig，含 max_tool_rounds/model）
3. **工具注册函数**（如 `register_summarizer_tools()`，在 `app-agent/src/tools.rs` 或专属 tools 文件）
4. **输出解析函数**（把 LLM 文本响应解析成结构化数据，类似 `parse_plan_from_response`）

### 6.2 角色识别 Agent（导入时跑）

> ✅ **P1 已实现（2026-06-15）**

- **crate**：`app-agent`
- **system prompt 常量**：`crates/app-agent/src/prompts/character_extractor.rs::CHARACTER_EXTRACTOR_SYSTEM_PROMPT`（含 JSON 输出格式示例）
- **config 构造**：同文件 `make_character_extractor_config()`（max_tool_rounds: 8）
- **用户消息拼装**：同文件 `build_character_extractor_user_msg(&Character)`（拼 description/personality/first_mes/alternate_greetings/character_book 全文）
- **工具注册**：同文件 `register_character_extractor_tools()`（注册 emit_characters 工具，handler `Ok(args)`）
- **编排入口**：`crates/app-agent/src/character_extractor.rs::extract_characters(runtime, character, mvu_schema, cancel)`（调 run_tool_loop）
- **输出解析**：同文件 `parse_character_definitions_from_response()` —— 5 层兜底（照搬 parse_plan_from_response 模式）：emit_characters 工具调用 / 整体 JSON 数组 / ```json 块 / 裸代码块 / 手写括号配平（match_braces，UTF-8 安全）
- **降级路径**：`domain::character::CharacterDefinition::fallback_from_character()`（识别失败建单角色 Protagonist）
- **输出**：`Vec<CharacterDefinition { name, persona_prompt, behavior_rules, base_backstory, group, role_type, variable_schema }>`
- **Tauri 命令**：`crates/tauri-app/src/lib.rs::extract_characters(source_character_id)`（跑识别 + 建卡 + 失败降级）
- **Mock 测试**：`infra-llm/src/mock_client.rs` 加了识别脚本（match_keyword="卡内角色识别"，插在 scripts 最前避开"角色"冲突）

**怎么改**：改 prompt → 编辑 `CHARACTER_EXTRACTOR_SYSTEM_PROMPT` 常量；改输出 schema → 同步改 `CharacterDefDto` + `dto_to_definition()`（character_extractor.rs）。

### 6.3 剧情总结 Agent（编剧后并行，独立）

> ✅ **P2 已实现（2026-06-15）**

- **crate**：`app-agent`（与后处理 Agent 同 crate，便于共享 runtime）
- **system prompt 常量**：`crates/app-agent/src/prompts/summarizer.rs::SUMMARIZER_SYSTEM_PROMPT`（本轮剧情总结助手，含内容优先级 6 项 + 200-500 字硬约束）
- **config 构造**：同文件 `make_summarizer_config()`（`AgentRole::Summarizer`，无工具）
- **用户消息拼装**：同文件 `build_summarizer_user_msg(final_text, scene_brief, turn)`
- **编排入口**：`crates/app-agent/src/summarizer.rs::run_summarizer(runtime, final_text, scene_brief, turn, cancel)`（调 run_tool_loop，纯文本输出）
- **工具**：无（纯文本输出，ToolRegistry::new()）
- **输出**：`String`（本轮摘要正文，200-500 字）
- **持久化**：`tauri-app/campaign_store::CampaignStore::add_summary()` → `data/round_summaries.json`（每轮一条，同 campaign_id + turn 覆盖）
- **Mock 测试**：`infra-llm/src/mock_client.rs` match_keyword="本轮剧情总结"

**关键设计**：本轮摘要（200-500 字，每轮一条）**≠ archiver 批量归档**（窗口溢出时把多条原文压成远记忆）。两者职责不重叠：summarizer 是「事件级原子单位」，archiver 是「长期压缩」。

**怎么改**：改 prompt → 编辑 `SUMMARIZER_SYSTEM_PROMPT` 常量。

### 6.4 后处理 Agent（编剧后并行，知识+变量+任务三合一）

> ✅ **P2 已实现（2026-06-15）**

- **crate**：`app-agent`
- **system prompt 常量**：`crates/app-agent/src/prompts/postprocess.rs::POSTPROCESS_SYSTEM_PROMPT`（含 JSON 输出格式示例，三大任务：角色知识/变量/任务）
- **config 构造**：同文件 `make_postprocess_config()`（`AgentRole::PostProcessor`，max_rounds: 8）
- **用户消息拼装**：同文件 `build_postprocess_user_msg(final_text, present_characters, variable_keys, turn, story_clock)`
- **工具注册**：同文件 `register_postprocess_tools()`（注册 emit_postprocess 工具，handler `Ok(args)`）
- **编排入口**：`crates/app-agent/src/postprocess.rs::run_postprocess(...)`（调 run_tool_loop）
- **输出解析**：同文件 `parse_postprocess_from_response()` —— 5 层兜底（照搬 parse_plan_from_response 模式）：emit_postprocess 工具调用 / 整体 JSON / ```json 块 / 裸代码块 / 手写括号配平（match_braces，UTF-8 安全）。**best-effort**：失败返回空 `PostProcessResult`，不报错
- **输出**：`PostProcessResult { knowledge_updates, variable_updates, task_updates }`
  - `knowledge_updates: Vec<CharacterKnowledgeUpdate>`（角色知识，四元 source 分类 + pinned）
  - `variable_updates: Vec<VariableUpdate>`（角色级 instance_id + 全局级 None）
  - `task_updates: Vec<TaskUpdate>`（新建伏笔 task_id=None + new_task / 改已有任务状态）
- **持久化**：`tauri-app` 的 `persist_postprocess_outcome()` 写进 `CampaignStore`：
  - knowledge → `data/knowledge.json`（update → entry，assign campaign_id + turn）
  - variables → `instances.json`（角色级，按 name 匹配 instance）/ `campaigns.json`（全局级，含 story_clock 推进）
  - tasks → `data/tasks.json`（新建 / 状态变化）
- **Mock 测试**：`infra-llm/src/mock_client.rs` match_keyword="后处理"

**并行编排**（关键）：`crates/app-agent/src/pipeline_postprocess.rs::run_postprocess_pipeline()` 用 `tokio::join!` 并发跑总结 + 后处理，**任一失败不影响另一个**（best-effort）。返回 `PostProcessOutcome { summary: Option<String>, post_process: Option<PostProcessResult> }`。

**怎么改**：改 prompt → 编辑 `POSTPROCESS_SYSTEM_PROMPT` 常量；改输出 schema → 同步改 `PostProcessDto` + `dto_to_result()`（postprocess.rs）。

---

## 7. 变量表接口（角色私有 + Campaign 全局）

> 每个角色绑定一张私有变量表（血量/状态/位置/好感度等），Campaign 维护一张全局表（故事时钟/天气/大势）。
> 参考 MVU 的 initvar + stat_data 机制：**卡定义 schema（含默认值），实例只存值**。

### 7.1 基础变量表（所有角色实例默认带）

**位置约定**：`crates/domain/src/variables.rs`（新增）

```rust
/// 变量字段定义（schema 层，来自卡 initvar 或基础表）
struct VariableField {
    key: String,            // "hp"
    label: String,          // "生命值"（UI 显示用）
    value_type: VariableType, // Int/Float/String/Bool
    default: serde_json::Value,
    description: Option<String>,
    group: Option<String>,  // "状态" / "关系" 分组（UI 折叠用）
}

enum VariableType { Int, Float, String, Bool, Json }
```

**基础表字段**（`default_character_variables()` 工厂函数返回，所有角色实例初始化时带）：

| key | label | 类型 | 默认 | 说明 |
|-----|-------|------|------|------|
| `hp` | 生命值 | Int | 100 | 战斗/受伤追踪 |
| `mp` | 体力/精力 | Int | 100 | 行动消耗 |
| `state` | 状态 | String | "正常" | 受伤/中毒/昏迷等文本 |
| `location` | 位置 | String | "" | 当前所在地 |
| `mood` | 情绪 | String | "平静" | 给导演/编剧参考 |
| `relationship_to_player` | 与玩家关系 | String | "陌生" | 好感/敌意 |
| `inventory` | 物品 | Json | [] | 道具列表 |

> 这只是起点。**MVU 卡导入时**，initvar 会覆盖/扩展这张表（加卡自定义字段）。**非 MVU 卡**也能在高玩模式手动加字段（见 §7.3）。

### 7.2 两级变量存储

**位置约定**：`crates/domain/src/character.rs`（CharacterInstance）+ `crates/domain/src/campaign.rs`（Campaign）

```rust
// 角色级（每角色一张）
struct CharacterInstance {
    // ... persona/behavior（固定）
    variables: Vec<VariableValue>,  // 实例存值，schema 来自 CharacterDefinition.variable_schema
}

struct VariableValue {
    key: String,
    value: serde_json::Value,
    last_updated_turn: u32,  // 哪轮改的（调试/回溯用）
}

// 全局级（Campaign 一张）
struct Campaign {
    // ...
    variables: Vec<VariableValue>,  // story_clock/weather/world_state 等
    active_variables: serde_json::Value,  // 聚合后每轮注入用（用完即弃）
}

// 角色卡定义 schema 层
struct CharacterDefinition {
    // ... persona 等
    variable_schema: Vec<VariableField>,  // 基础表 + 卡 initvar 扩展
}
```

**注入用 active_variables 的生成**（每轮后处理 Agent 跑完后聚合）：
```rust
fn build_active_variables(campaign: &Campaign, present_characters: &[&CharacterInstance]) -> serde_json::Value {
    // 聚合：全局变量 + 在场角色的变量 → 一份扁平 JSON，放末尾 user message
}
```

### 7.3 变量表修改接口（高玩模式 + 用户手动）

**位置约定**：`crates/tauri-app/src/lib.rs`（新增 Tauri 命令）

| 命令 | 作用 |
|------|------|
| `get_variable_schema(character_def_id)` | 查某角色定义的 schema（字段列表） |
| `update_variable_field(character_def_id, field)` | 改字段定义（label/type/default/description）—— 改的是**卡级定义**，影响该卡所有新档 |
| `add_variable_field(character_def_id, field)` | 给某角色加字段（用户手动加，或给非 MVU 卡用） |
| `delete_variable_field(character_def_id, key)` | 删字段 |
| `get_character_variables(campaign_id, instance_id)` | 查某实例的当前变量值 |
| `set_character_variable(campaign_id, instance_id, key, value)` | 手动改某实例的值（调试/纠错） |
| `get_campaign_variables(campaign_id)` / `set_campaign_variable(...)` | 全局变量读写 |

**怎么改基础表**：改 `crates/domain/src/variables.rs` 的 `default_character_variables()` 工厂函数，加减字段。改后所有新创建的角色实例自动带新字段。

### 7.4 变量更新流程（谁改值）

| 触发 | 谁改 | 改什么 |
|------|------|--------|
| 每轮后处理 Agent | 后处理 Agent 解析成文 `_.set` 指令 | 自动更新角色/全局变量（见 §7 MVU 接口） |
| 用户手动（高玩模式） | 前端调 set_*_variable 命令 | 调试/纠错 |
| 共享 WebView 跑卡 JS | JS 算出变量副作用 | 回调 Rust 写值（重 DOM 卡） |

### 7.5 变量注入提示词模板

**位置约定**：`crates/domain/src/variables.rs` 的 `render_variables_for_injection(vars) -> String`

```text
【当前世界状态】
故事时间：{story_clock}
天气：{weather}
（其他全局变量）

【在场角色状态】
林医生：生命 80 / 状态 受伤 / 位置 急诊室 / 情绪 紧张
陈警官：生命 100 / 状态 警觉 / 位置 现场外
```

这段拼进末尾 user message（不进 system，保证 cache 命中）。

---

## 8. MVU 变量接口（兼容层）

> MVU 协议层是 §7 变量体系的「数据来源之一」——MVU 卡的 initvar 解析后填进 CharacterDefinition.variable_schema，运行时和普通变量统一处理。
>
> **核心原则（对话推敲后定型）**：
> ① 渲染 / 逻辑分离——渲染是数据绑定（声明式），逻辑是 tool-call（翻译式）
> ② 翻译 = JS 逻辑 → ToolCallSpec（不是 → Rust 结构）
> ③ 元素级混合——能翻译的翻译，不能的保留 JS 运行时执行

### 8.1 翻译产物（MvuTranslation，Meta Agent 导入时产出）

**位置**：`crates/domain/src/mvu_translation.rs`（✅ 已实现 2026-06-16，含 CardComplexityReport + score_card_complexity 启发式打分）

```rust
struct MvuTranslation {
    variable_schema: Vec<VariableField>,    // 卡 initvar → 变量定义
    ui_bindings: Vec<UiBinding>,            // UI 元素 ↔ 变量（声明式绑定）
    update_rules: Vec<String>,              // 自然语言规则（注入后处理 Agent）
    interactions: Vec<InteractionMapping>,  // JS 事件 → ToolCallSpec
    fallback_fragments: Vec<FallbackFragment>, // 翻译不了的 JS（运行时执行）
}

struct UiBinding { element: String, variable_key: String, display: BindingDisplay }
enum BindingDisplay { Bar { max: f64 }, Text, Tag, Icon { mapping: HashMap<String,String> } }

struct InteractionMapping { element_label: String, actions: Vec<ToolCallSpec> }
struct ToolCallSpec { tool_name: String, args: serde_json::Value }

enum InteractionAction {
    ModifyVariable { key, value_expr },
    TriggerNextTurn { hint: String },
    Multi(Vec<InteractionAction>),
    RunOriginalJs { js_snippet: String, description: String },
}

struct FallbackFragment { description: String, js_snippet: String, reason: String }
```

**怎么改翻译规则**：高玩模式直接改 `update_rules` 文本 / `interactions` 映射，覆盖 Meta Agent 自动翻译结果。

### 8.2 运行时三路执行

| 路 | 触发 | 干什么 | 位置 |
|----|------|--------|------|
| 渲染 | 每轮成文后 | 前端拿 `ui_bindings` + 变量值，原生画状态栏 | 前端组件 |
| 后处理 | 编剧后并行 | 后处理 Agent 读 `update_rules`，调 tool-call 改变量 | `app-postprocess` |
| 用户交互 | 用户点击 | 前端按 `interactions` 调对应 tool；翻译不了的走 `RunOriginalJs` | 前端 + runtime |

### 8.3 Meta Agent 导入时五合一分析

**位置**：`crates/app-meta/src/mvu_import.rs`（✅ 已实现 2026-06-16，analyze_mvu_card 编排 + parse_mvu_translation_from_response 5 层兜底 + classify_st_preset_with_llm ST 分类）

一次调用产出 `MvuTranslation`（变量 schema + UI 绑定 + 规则 + 交互映射 + 兜底片段）。元素级判定能翻译/不能翻译，不强制全卡统一。置信度低的标注"建议验证"。

> Meta Agent 的 system prompt 位置：`crates/app-meta/src/prompts/mvu_analyzer.rs`（✅ 已实现，MVU_ANALYZER_SYSTEM_PROMPT）
> Meta 多轮对话框架：`crates/app-meta/src/meta_conversation.rs`（✅ 已实现，chat 编排 + 工具结果结构化）
> Meta 诊断 prompt：`crates/app-meta/src/prompts/meta_agent.rs`（✅ 已实现，META_AGENT_SYSTEM_PROMPT）

### 8.4 兜底执行（翻译不了的 JS）

**位置**：`crates/infra-plugin-host/src/mvu_runtime.rs`（⚠️ 桩状态 2026-06-16，StubMvuRuntime 全部 NotImplemented；真实 WebView 执行待下一轮）

| 方案 | 适用 | 触发 |
|------|------|------|
| QuickJS（嵌入式） | 零星 JS 片段 | `InteractionAction::RunOriginalJs` |
| 共享 WebView（全局常驻） | 大量 JS + DOM 依赖 | `fallback_fragments` 非空 + 有 DOM 调用 |

共享 WebView 模型：全局一个隐藏 WebView，加载卡 JS 一次（O(1) 内存）← Rust 推 stat_data + 成文 → JS 算状态栏/副作用 → Rust 收结果。

**JS 桥 API**（卡 JS 调用，宿主提供）：
- `Mvu.parseMessages()` / `Mvu.setData()` — MVU 协议（WebView 调 Rust）
- `document.*` / `window.*` / `$(...)` — 真实 DOM

**依赖**：兜底执行依赖插件运行时（§8 of TECHNICAL_DESIGN）基础设施。落地顺序：先做翻译 + 原生路径，WebView 兜底后做。

---

## 9. 叙事计划系统接口（任务追踪 / 长程一致性）

> 解决"导演忘记三个月后的伏笔"问题。每轮总结 Agent 抽取/比对任务，接近触发时注入下一轮导演提示词。

### 9.1 数据结构

**位置约定**：`crates/domain/src/story_task.rs`（新增）

```rust
struct StoryTask {
    id: Id,
    campaign_id: Id,
    title: String,                    // "老王三个月后复仇"
    description: String,
    trigger: Vec<TaskTrigger>,        // 多个触发 OR 关系，任意满足即提醒
    status: TaskStatus,               // Pending/Active/LikelyCompleted/Completed/Abandoned
    completion_confidence: Option<f32>, // LikelyCompleted 时的置信度
    created_turn: u32,
    related_characters: Vec<Id>,
    source: TaskSource,               // UserPlanned / ExtractedFromNarrative
    injected_turns: Vec<u32>,         // 已在哪些轮注入（防重复/统计）
}

enum TaskTrigger {
    Event(String),                    // "角色X得知真相" —— 后处理 Agent 判断
    TurnReminder(u32),                // 第 N 轮提醒 —— 倒计时
    StoryTime(String),                // "第2年6月" —— 故事时钟匹配
    Manual,                           // 只手动激活
}

enum TaskStatus { Pending, Active, LikelyCompleted, Completed, Abandoned }
enum TaskSource { UserPlanned, ExtractedFromNarrative }
```

### 9.2 任务来源（两条录入路径）

| 来源 | 谁录入 | 触发 |
|------|--------|------|
| **用户显式规划** | 用户在前端建（高玩模式） | "我希望三个月后老王复仇"——UI 命令建任务 |
| **叙事中自然产生** | 后处理 Agent 抽取 | 读成文发现伏笔，自动建任务，source=ExtractedFromNarrative |

### 9.3 每轮比对（后处理 Agent 职责，合并进一次调用）

后处理 Agent 每轮除了抽知识/更新变量，还要：
1. **比对触发**：哪些 Pending/Active 任务的 trigger 已满足？（事件靠 LLM 判断，轮次/时钟确定性比对）
2. **完成检测**：哪些任务可能完成？（输出置信度，不直接标 Completed）
3. **抽取新伏笔**：成文里有没有新埋的任务？（建新任务）

输出合并进 `PostProcessResult`：
```rust
struct PostProcessResult {
    knowledge_updates: Vec<CharacterKnowledgeUpdate>,
    mvu_updates: Option<MvuVariableUpdate>,
    task_updates: Vec<TaskUpdate>,        // 新增
}

struct TaskUpdate {
    task_id: Option<Id>,                  // None = 新建任务
    new_status: TaskStatus,
    confidence: Option<f32>,
    new_task: Option<StoryTask>,          // task_id=None 时填
}
```

### 9.4 任务注入（确定性查表，零 LLM 成本）

**位置约定**：`crates/app-pipeline/src/lib.rs` 的 `build_director_user_msg()` 增加一段

在导演 user message 末尾（cache 友好的位置）追加：
```text
【即将触发的任务】
- 老王三个月后复仇（故事时间接近：当前第2年5月，目标第2年6月）
- 陈警官得知真相（事件触发：本轮角色X提到了证据）
```

注入规则（在 `build_director_user_msg` 里实现，**P2 已接入**）：
- 遍历该 campaign 的 Pending/Active 任务（`ctx.pending_tasks`，从 `CampaignStore::list_tasks` 加载）
- 任一 trigger 满足（`render_tasks_for_injection` 过滤）→ 注入
- 已 Completed/Abandoned → 跳过
- LikelyCompleted（置信度 > 0.8）→ 提示用户确认，不自动注入（避免误判消失）
- 零 LLM：纯确定性查表（`ctx.turn` / `ctx.story_clock` 比对 trigger）

### 9.5 任务的 Tauri 命令接口（用户手动管理）

> ✅ **P2 已实现（2026-06-15）**

**位置**：`crates/tauri-app/src/lib.rs`

| 命令 | 作用 | 实现状态 |
|------|------|---------|
| `list_character_knowledge(campaign_id, character_id?)` | 列角色可见信息（character_knowledge，P2 新增） | ✅ |
| `list_tasks(campaign_id, status_filter?)` | 列任务（按状态筛：pending/active/likely_completed/completed/abandoned） | ✅ |
| `create_task(campaign_id, title, description, triggers, created_turn?)` | 用户手动建任务 | ✅ |
| `complete_task(task_id)` | 手动标记完成（覆盖 Agent 判断） | ✅ |
| `abandon_task(task_id)` | 放弃任务 | ✅ |
| `list_round_summaries(campaign_id)` | 列本轮剧情摘要（按 turn 升序，P2 新增） | ✅ |
| `update_task(task_id, change)` | 改任务（标题/触发/描述） | ⏳ 未做（用 create + abandon 替代） |
| `confirm_likely_completed(task_id, accept)` | 确认/驳回 LikelyCompleted 提示 | ⏳ 未做（用 complete_task 替代） |

---

## 10. cache 友好消息布局接口（MessageLayout）

> ✅ **§22 已落地（2026-06-16）**：导演/编剧/子 Agent 全部走 `run_tool_loop_with_layout`（`app-agent/runtime.rs:313`），消费 `MessageLayout`。蓝灯世界设定进 system、对话历史进独立 history 段、意图/任务/场景进 tail。
>
> LLM 的 KV cache 按前缀字节匹配。稳定前缀命中 cache 省钱省延迟，易变内容必须压尾。
> MessageLayout 是类型层护栏：编译期强制三段分离，禁止易变内容污染前缀。

### 10.1 三段布局

```
[1] system（稳定，整个会话不变）
    role_directive + 模块 + 蓝灯世界设定 + 工具说明
    ← cache 全命中
    
[2] 历史消息（稳定前缀，逐轮 append，写入后永不改）
    [user1][asst1]...[userN-1][asstN-1]
    ← cache 全命中
    
[3] 当轮 user message（易变，每轮新建，用完即弃）
    写作意图 + 变量 + 故事时钟 + 任务提醒 + 在场角色状态
    ← 只影响这条
```

### 10.2 MessageLayout 抽象（编译期护栏）

**位置约定**：`crates/domain/src/message_layout.rs`（新增）

```rust
/// 强制三段布局，禁止易变内容污染前缀
pub struct MessageLayout {
    stable_system: String,            // [1] 只能填一次
    stable_history: Vec<Message>,     // [2] 只接受不可变引用追加
    volatile_tail: VolatileTail,      // [3] 唯一允许每轮变动
}

impl MessageLayout {
    pub fn build() -> MessageLayoutBuilder { ... }
    pub fn into_messages(self) -> Vec<ChatMessage> { ... }
}

// Builder 用类型状态机保证顺序：
// build().system(s).history(h).tail(t)
// 调 .tail() 后不能再 .system()/.history()
```

**核心约束**：
- `.system()` / `.history()` 只接受 `&str` / `&[Message]`（不可变）
- `.tail()` 是唯一能塞变量的地方
- 想"在 history 中间插消息"？编译器拒绝，必须走显式 API

### 10.3 子 Agent 的布局（cache 命中率更高）

子 Agent 比 主 Agent 更适合 cache——persona/behavior 整个 campaign 不变：

```
[1] system（整个 campaign 稳定）
    persona + behavior + base_backstory + 工具说明
    ← cache 全命中（最稳）

[2] pinned 知识（慢变）
    backstory 知识 + 重大揭示（pinned=true 的 knowledge）
    ← 多轮稳定，cache 命中

[3] 当轮 user message
    导演情境 + recent_window + 该角色当前变量 + 相关任务提醒
    ← 每轮新建
```

### 10.4 pinned 知识（控制稳定前缀长度）

**位置约定**：`crates/domain/src/character.rs` 的 `CharacterKnowledgeEntry` 加 `pinned: bool` 字段

```rust
struct CharacterKnowledgeEntry {
    // ... 现有字段
    pinned: bool,  // true = 进 system 慢变层；false = 走向量检索/末尾
}
```

**pin 规则**：
- backstory 来源的知识 → 自动 pin
- 后处理 Agent 判断"重大事件"（如重大揭示）→ 标记 pin
- 用户手动 pin/unpin（高玩模式）
- pinned 知识总量有上限（如 2000 token），超了挤掉最老的，防止 system 膨胀

### 10.5 用 MessageLayout 的好处

1. **防回归**：cache 破坏是 silent bug（不报错只变慢）。类型约束在编译期挡住
2. **自文档**：看一眼 layout 就知道哪些稳定、哪些易变
3. **可测试**：CI 断言"system + history 跨轮 byte 一致"

### 10.6 改布局要注意什么

- 改 [1] system 内容（如调 prompt 模块）→ 该轮 cache 全失效，**之后重建**。改前想清楚
- 改 [2] 历史 → **写入后永不修改**。edit_variant 是建新 variant 不是改旧值（现有设计已符合）
- 软删/discarded 消息 → **不能物理删除**，只标记（否则后面前缀变了）

---

## 11. 修改 prompt 的 Checklist（每次改都过一遍）

- [ ] 改的是哪个 Agent？（导演/编剧/子Agent/新增的三个）
- [ ] 改常量还是改函数？常量改 `*_SYSTEM_PROMPT`，动态拼装改对应函数
- [ ] 改了输出格式 → 输出解析函数（`parse_*_from_response`）同步改了吗？
- [ ] 改了子 Agent 上下文 → 两个 `format_*_context` 函数（pipeline + runtime）都改了吗？
- [ ] 改了工具集 → `register_*_tools` 函数改了吗？工具说明（tool_directives）是自动生成的，不用手改
- [ ] **改的内容会破坏 cache 吗？** 易变内容必须进 MessageLayout.tail()，不能进 system/history
- [ ] 改完跑测试：`cargo test -p storyforge-app-pipeline` / `cargo test -p storyforge-app-agent`

---

## 12. 常见修改场景速查

### 场景 A：「导演老是不按我的想法规划」

改 `DIRECTOR_SYSTEM_PROMPT`（`lib.rs:53`）。在「判断本场戏」那段加你的规则，比如「优先推进用户上一轮提到的人物」「不要引入超过 3 个新角色」。

### 场景 B：「子 Agent 出戏，说话风格不像角色」

两层都要看：
1. 角色设定本身够不够细 → 看 `ContextPackage.character_brief`（来自角色卡 description/personality）
2. 系统提示词约束够不够 → 改 `SUBAGENT_SYSTEM_PROMPT_TEMPLATE`（`lib.rs:79`），加「严格保持角色语气，不要出现叙事腔」

### 场景 C：「编剧写得太短/太长」

改 `EDITOR_SYSTEM_PROMPT` 加字数约束，或挂「字数控制」模块（`prompt_module.rs:253`，已内置）。

### 场景 D：「想加个新工具给导演用」

1. 在 `crates/app-agent/src/tools.rs` 写工具实现函数
2. 在 `register_director_tools()` 里注册
3. 工具说明（tool_directives）会自动生成进 system prompt

### 场景 E：「想全局换文风」

改 `preset_modules()`（`prompt_module.rs:206`）里的「白描」模块内容，或新增一个模块绑到编剧。导演/编剧已接入模块系统（§5.4），可在高玩模式 UI 点按钮切换（AgentConfigCard）。

### 场景 F：「想给角色加个变量字段（如『疲劳度』）」

1. 卡级（影响该卡所有新档）：调 `update_variable_field` / `add_variable_field` 命令，或改 `CharacterDefinition.variable_schema`
2. 全局默认（所有角色都加）：改 `crates/domain/src/variables.rs` 的 `default_character_variables()` 工厂

### 场景 G：「想让某个伏笔在特定时间点提醒导演」

1. 前端调 `create_task` 建任务，trigger 选 `StoryTime("第2年6月")` 或 `TurnReminder(30)`
2. 系统每轮自动比对，到点注入导演 user message 末尾
3. 完成后调 `complete_task`，或让后处理 Agent 标 LikelyCompleted 待你确认

---

## 13. Meta Agent（配置调试助手，独立于写作流水线）

> ✅ **已实现（2026-06-16）**：诊断 + Patch 系统 + 多轮流式对话 + MVU 五合一分析 + ST 预设分类。对应设计 §9、§19.4。
> **未实现**：插件生成（§9.2）、ST 预设→模块库闭环（§9.4）。
>
> **关键定位**：Meta Agent **不参与写作流水线**——不读 `current_cancel`、不碰 pipeline、不接触 API key 明文。自建 `AgentRuntime` + 每轮新建 `ToolRegistry`。安全约束：patch 必须用户点「采纳」才执行；API key 永不经过 Meta Agent。

### 13.1 多轮对话框架（流式）

**crate**：`crates/app-meta/src/meta_conversation.rs`

| 符号 | 行号 | 职责 |
|------|------|------|
| `META_AGENT_SYSTEM_PROMPT` | `prompts/meta_agent.rs:28` | 角色定位：「StoryForge 配置调试助手，不参与写作流水线，帮诊断/整理预设/提议 Patch，不能直接修改，改动必须用户采纳」 |
| `make_meta_agent_config()` | `prompts/meta_agent.rs:51` | AgentConfig（`AgentRole::Meta`，max_rounds: 8） |
| `MetaSession` | `meta_conversation.rs` | 跨工具调用共享的诊断数据源：`character: Mutex<Option<Arc<Character>>>` + `world_info: Mutex<Option<Arc<WorldInfoBook>>>` + `patches: PatchStore` |
| `MetaConversation` | `meta_conversation.rs` | 内存态对话（重启清空）：`messages: Vec<MetaMessage>` + `history_summary: Vec<String>`（逐轮摘要，保留最近 6 轮喂下一轮） |
| `chat(...)` | `meta_conversation.rs:146` | 流式编排：make_meta_agent_config + build_meta_user_msg（拼 history_summary）+ register_meta_runtime_tools + `runtime.run_tool_loop_streaming(progress_tx, None)`。返回 `MetaTurn { agent_message, new_patch }` |

**Tauri 命令**：`meta_start_conversation`（建对话）/ `meta_chat`（流式，推 `meta_progress` 事件）/ `meta_get_conversation`。

> ⚠️ **两套 patch 存储**：`MetaSession.patches`（app-meta 内部）与 `AppState.meta_patches`（tauri 侧）是两份。`meta_chat` 结束把 session 新 patch 克隆进 AppState；`meta_accept_patch`/`meta_dismiss_patch` 只改 AppState.meta_patches，不反向同步回 session（session 是临时态，目前无害，但属潜在漂移点）。

### 13.2 诊断工具集（只读）

**位置**：`crates/app-meta/src/lib.rs` + `register_meta_runtime_tools`（meta_conversation.rs）

| 工具 | 函数 | 做什么 |
|------|------|--------|
| `meta_inspect_world_info` | `inspect_world_info()`（lib.rs:95） | 检查蓝灯关键词冲突（两两交集）+ 孤立条目（绿灯无 keys）。返回 `WorldInfoReport` |
| `meta_inspect_character` | `inspect_character()`（lib.rs:143） | 检查字段非空 + first_mes 占位符（"【首页】"）。返回 `CardReport` |
| `meta_propose_patch` | session handler | **写**：`session.patches.propose(desc, actions)` 生成 Patch（不直接执行） |

### 13.3 Patch 系统（提议-采纳两阶段）

**位置**：`crates/app-meta/src/lib.rs`

```
LLM propose ──▶ PatchStore.propose（生成 uuid, applied=false）
                   │  同步进 AppState.meta_patches（前端可见）
                   ▼
用户采纳 ──▶ meta_accept_patch 命令（tauri-app/src/lib.rs:2367）
                   │  execute_patch（app-meta 纯函数，传 PatchContext）
                   │  ① 改 tool_ctx 内存世界书
                   │  ② 持久化 CharacterStore：全局条目写回所有卡 + 非全局按 keys 指纹匹配回原卡
                   │  ③ patch.applied = true
                   ▼
用户忽略 ──▶ meta_dismiss_patch（retain 移除）
```

- **PatchAction**：`Create{target,data}` | `Update{target,field,value}` | `Delete{target}`。`target` 格式 `world_info[0]` / `character.personality`。当前只支持 `world_info` / `character` 两种 kind。
- **历史 bug（2026-06-16 已修）**：曾把合并世界书写回最后一张卡 + `is_global` 硬编码 false。现 `is_global` 从 route 推导（Constant/Both=true）+ keys 匹配替代 content 匹配。

### 13.4 MVU 五合一分析（手动触发，设计 §19.4）

**位置**：`crates/app-meta/src/mvu_import.rs` + `prompts/mvu_analyzer.rs`

| 符号 | 行号 | 职责 |
|------|------|------|
| `MVU_ANALYZER_SYSTEM_PROMPT` | `prompts/mvu_analyzer.rs:27` | 「卡内状态栏分析助手，做五合一分析产出 MvuTranslation」 |
| `make_mvu_analyzer_config()` | `prompts/mvu_analyzer.rs:96` | AgentConfig（emit_mvu_translation 工具） |
| `analyze_mvu_card(...)` | `mvu_import.rs:41` | 编排：①score_card_complexity 启发式打分（document./innerHTML/script 阈值）→ CardComplexityReport ②extract_mvu_schema_from_extensions（P1 字段级 schema）③纯数据短路（PureData 无字段 → pure_data_fallback 省 LLM）④LLM 五合一（run_tool_loop 非流式）⑤parse_mvu_translation_from_response（5 层兜底）⑥失败降级 pure_data_fallback |
| `classify_st_preset_with_llm(...)` | `mvu_import.rs:529` | ST 预设逐条 LLM 分类（增强启发式 bridge）。system prompt：`ST_CLASSIFY_SYSTEM_PROMPT`（mvu_import.rs:655） |

**五产物（MvuTranslation）**：`variable_schema` / `ui_bindings`（BindingDisplay: bar/text/tag/icon）/ `update_rules`（注入后处理 Agent）/ `interactions`（用户点击→动作）/ `fallback_fragments`（需 WebView 的片段）。`routing`：native/hybrid；LLM 标 native 却有 fallback_fragments 会自动纠正为 hybrid。

**持久化**：`StoredMvuTranslation` → `data/mvu_translations.json`（按 source_character_id 去重）。

> ⚠️ **共享 WebView 是桩**：`infra-plugin-host/src/mvu_runtime.rs` 的 `StubMvuRuntime` 全 NotImplemented，重 DOM 卡（缄默之秋1.4 类，document.×179）的 fallback_fragments 无法执行——已知限制。

### 13.5 9 个 Meta/MVU Tauri 命令速查

| 命令 | 作用 | 推事件？ |
|------|------|---------|
| `meta_start_conversation` | 新建 MetaConversation（uuid） | 否 |
| `meta_chat` | 跑一轮流式对话（诊断/ST 分类/提议 Patch） | **是**（meta_progress） |
| `meta_get_conversation` | 取对话历史 | 否 |
| `meta_list_pending_patches` | 列待采纳 patch | 否 |
| `meta_dismiss_patch` | 忽略 patch | 否 |
| `meta_accept_patch` | 执行 patch（改世界书 + 写 CharacterStore） | 否 |
| `meta_analyze_mvu_card` | MVU 五合一分析（手动触发） | 否 |
| `meta_list_mvu_translations` / `meta_get_mvu_translation` | 查 MVU 翻译 | 否 |
| `meta_classify_st_preset` | ST 预设 LLM 分类（不持久化） | 否 |

### 13.6 怎么改 Meta Agent 的 prompt

| 想改 | 改哪 |
|------|------|
| Meta 对话的角色定位/诊断行为 | `META_AGENT_SYSTEM_PROMPT`（`prompts/meta_agent.rs:28`） |
| MVU 分析的判定逻辑/输出格式 | `MVU_ANALYZER_SYSTEM_PROMPT`（`prompts/mvu_analyzer.rs:27`）。改输出 schema 要同步改 `parse_mvu_translation_from_response` 的 5 层兜底 |
| ST 预设分类规则 | `ST_CLASSIFY_SYSTEM_PROMPT`（`mvu_import.rs:655`） |
| Patch 执行逻辑 | `execute_patch`（lib.rs，PatchContext 处理 Create/Update/Delete） |
| 诊断检查项 | `inspect_world_info` / `inspect_character`（lib.rs） |

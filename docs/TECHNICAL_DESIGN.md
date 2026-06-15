# 技术方案设计文档

> 项目代号：**StoryForge**（暂定，可改）
> 基于 `INTENT.md` v3（D1–D44 全部锁定）编写
> 状态：设计稿，待评审 → 评审通过后进入 M0 开发。§16-§20 为 2026-06-15 新增（角色子 Agent / Campaign / MVU 兼容方向）。

---

## 0. 文档导航

| 章节 | 内容 |
|------|------|
| §1 | 系统全景与设计原则 |
| §2 | 整体架构图（分层） |
| §3 | 写作流水线详细设计（2主+N子） |
| §4 | 资料源体系（角色卡/世界书/预设/记忆） |
| §5 | Rust 数据模型（核心 struct） |
| §6 | 模块拆分（crate 级） |
| §7 | 记忆系统原生实现（参考 shujuku） |
| §8 | 插件运行时设计 |
| §9 | Meta Agent 设计 |
| §10 | 前端架构（普通/高玩双视图） |
| §11 | 安卓部署与构建 |
| §12 | 日志查看系统 |
| §13 | 里程碑规划（M0→M5） |
| §14 | 首周任务清单 |
| §15 | 技术风险与对策 |
| **§16** | **角色子 Agent 与信息隔离（D30-D31，新增）** |
| **§17** | **多角色卡模型与 Campaign（D32-D38，新增）** |
| **§18** | **角色知识系统与后处理流水线（D39-D41，新增）** |
| **§19** | **MVU 变量框架原生兼容（D42-D43，新增）** |
| **§20** | **Agent 接口索引（D44，新增，详见 AGENT_INTERFACES.md）** |
| **§21** | **叙事计划系统（任务追踪/长程一致性，D45，新增）** |
| **§22** | **cache 友好消息布局（D46，新增）** |
| **§23** | **变量层级体系（D47-D48，新增）** |

---

## 1. 系统全景与设计原则

### 1.1 产品定位（一句话）
**仅安卓**的 AI 多 Agent 协作写作 App：用「导演+编剧+子Agent」的显式编排替代 ST 的「单 prompt + 提示词注入」范式；导入兼容 ST 角色卡/世界书/预设；后端原生实现小白X 级别的总结/推进/向量化；自带插件运行时（自有 API）；附 Meta Agent 做配置调试。

### 1.2 五条设计原则

| # | 原则 | 含义 |
|---|------|------|
| P1 | **Agent 原生，非注入原生** | 所有资料（角色卡/世界书/预设）都是 Agent 的**工具调用对象**，不是 prompt 拼接素材 |
| P2 | **后端编排，前端展示** | 多 Agent 并行/取消/流式合并都在 Rust 后端，前端只渲染状态 |
| P3 | **数据兼容，运行不兼容** | 能导入 ST 数据格式，但不运行 ST 插件（除角色卡 iframe 渲染） |
| P4 | **双视图渐进披露** | 默认普通视图（隐藏复杂度），高玩模式解锁全部 |
| P5 | **插件用自有 API** | 自建运行时，插件为我们写，不模拟 ST |

### 1.3 不做的事（明确排除）
- ❌ iOS / 桌面端（D15）
- ❌ 真 ST 插件运行（D6/D17）
- ❌ ST 提示词注入范式（蓝灯/绿灯拼 prompt）
- ❌ 全局脚本兼容（shujuku 那类，改原生替代）

---

## 2. 整体架构图

```
┌─────────────────────────────────────────────────────────────────┐
│                        Android App (APK)                         │
│                                                                  │
│  ┌───────────────────────────────────────────────────────────┐  │
│  │  前端层（WebView: Tauri v2 / 或原生 WebView）              │  │
│  │  ┌──────────────┐  ┌──────────────┐  ┌────────────────┐  │  │
│  │  │ 普通用户视图  │  │ 高玩视图     │  │ Meta Agent 对话│  │  │
│  │  │ (默认)       │  │ (可切换)     │  │ 框             │  │  │
│  │  └──────┬───────┘  └──────┬───────┘  └────────┬───────┘  │  │
│  │         └─────────┬───────┴────────────────────┘          │  │
│  │                   │ Tauri IPC (invoke)                     │  │
│  │         ┌─────────▼─────────────────────────────┐         │  │
│  │         │  角色卡渲染区（iframe 沙箱 + API 桥）  │         │  │
│  │         │  插件运行时（iframe 沙箱 + API 桥）    │         │  │
│  │         └───────────────────────────────────────┘         │  │
│  └───────────────────────────────────────────────────────────┘  │
│                              ↕ Tauri IPC                         │
│  ┌───────────────────────────────────────────────────────────┐  │
│  │  后端层（Rust, Tauri v2 Android target）                   │  │
│  │                                                            │  │
│  │  ┌─────────────────────────────────────────────────────┐  │  │
│  │  │  Presentation（Tauri commands）                     │  │  │
│  │  │  写作命令 / 资料命令 / Meta命令 / 插件命令          │  │  │
│  │  └─────────────────────────────────────────────────────┘  │  │
│  │  ┌─────────────────────────────────────────────────────┐  │  │
│  │  │  Application（服务编排）                            │  │  │
│  │  │  ┌───────────┐ ┌───────────┐ ┌───────────────────┐ │  │  │
│  │  │  │ 写作流水线│ │ 记忆系统  │ │ Agent 运行时      │ │  │  │
│  │  │  │ Orches-  │ │ (归档/召回│ │ (委派/工具/取消)  │ │  │  │
│  │  │  │ trator   │ │ /rerank)  │ │                   │ │  │  │
│  │  │  └───────────┘ └───────────┘ └───────────────────┘ │  │  │
│  │  └─────────────────────────────────────────────────────┘  │  │
│  │  ┌─────────────────────────────────────────────────────┐  │  │
│  │  │  Domain（领域模型，纯逻辑无 IO）                    │  │  │
│  │  │  Character / WorldInfo / Preset / Memory /          │  │  │
│  │  │  AgentProfile / Plan / Regex / PluginManifest       │  │  │
│  │  └─────────────────────────────────────────────────────┘  │  │
│  │  ┌─────────────────────────────────────────────────────┐  │  │
│  │  │  Infrastructure（实现层）                           │  │  │
│  │  │  ┌──────────┐ ┌──────────┐ ┌──────────┐ ┌────────┐ │  │  │
│  │  │  │ 文件持久化│ │ LLM HTTP │ │ 向量库   │ │ 嵌入API│ │  │  │
│  │  │  │ (JSON)   │ │ Client池 │ │ (本地)   │ │ (远程) │ │  │  │
│  │  │  └──────────┘ └──────────┘ └──────────┘ └────────┘ │  │  │
│  │  │  ┌──────────┐ ┌──────────┐ ┌──────────┐            │  │  │
│  │  │  │ ST 卡导入│ │ 正则引擎 │ │ 插件沙箱 │            │  │  │
│  │  │  │ 解析器   │ │ (regress)│ │ 宿主     │            │  │  │
│  │  │  └──────────┘ └──────────┘ └──────────┘            │  │  │
│  │  └─────────────────────────────────────────────────────┘  │  │
│  └───────────────────────────────────────────────────────────┘  │
│                              ↕                                   │
│  ┌───────────────────────────────────────────────────────────┐  │
│  │  外部服务（远程）                                         │  │
│  │  LLM API (OpenAI/Claude/...) · 嵌入 API · (可选)rerank   │  │
│  └───────────────────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────────────┘
```

**架构借鉴 TT 的 DDD 四层**（Presentation/Application/Domain/Infrastructure），但**代码全新**，砍掉所有 iOS/桌面/LAN 同步/系统托盘等用不上的模块。

---

## 3. 写作流水线详细设计（2主+N子）

### 3.1 流水线状态机

```
                    用户提交写作意图
                          │
                          ▼
                ┌─────────────────┐
                │  IDLE           │
                └────────┬────────┘
                         │ start_writing(intent)
                         ▼
                ┌─────────────────┐    失败/取消
                │  DIRECTING      │───────────────┐
                │  导演 Agent 运行│               │
                │  · 解析意图     │               │
                │  · 工具:查资料  │               ▼
                │  · 输出 Plan    │         ┌──────────┐
                └────────┬────────┘         │ ABORTED  │
                         │ plan_ready       └──────────┘
                         ▼
                ┌─────────────────┐
                │  DELEGATING     │  ← 并发核心
                │  派发 N 个子Agent│
                │  各自独立上下文  │
                │  各自可取消      │
                └────────┬────────┘
                         │ all_subagents_done (或部分完成)
                         ▼
                ┌─────────────────┐
                │  EDITING        │
                │  编剧 Agent 运行│
                │  · 收集子产出   │
                │  · 工具:查向量  │
                │  · 输出成文     │
                └────────┬────────┘
                         │ draft_ready
                         ▼
                ┌─────────────────┐
                │  REVIEW         │  用户预览/编辑/采纳
                └────────┬────────┘
                         │ accept
                         ▼
                ┌─────────────────┐
                │  COMMITTED      │  写入对话历史
                │  · 触发记忆归档 │  · 触发总结(可选)
                └─────────────────┘
```

### 3.2 三个 Agent 的职责契约

#### 导演 Agent（Director）
```rust
// 系统提示词核心（可被预设覆盖，D11）
"你是写作导演。用户给你写作意图，你要：
1. 调用 search_world_info / search_vectors / get_character 了解可用素材
2. 决定本场戏出场哪些角色、各自要做什么
3. 输出结构化的 Plan：每个子Agent的 {角色, 任务, 专属上下文包}
4. 不要自己写正文"
```
**工具集**：`search_world_info`, `get_character`, `search_vectors`(向量记忆), `get_recent_summary`, `emit_plan`
**输出**：`Plan { subagents: [SubagentTask { character_id, brief, context_package }] }`

#### 子 Agent（Subagent，N 个并行实例）
```rust
"你是角色 {name}。根据导演给你的任务和专属上下文，演出你这个角色在这场戏的
行为/对白/心理。只演你自己，不要替别人说话。输出纯表演，不要解释。"
```
**工具集**：`search_vectors`(只读自己的上下文包+向量), `get_character`(只读自己)
**约束**：不互相通信，不跑正则（D12），独立取消
**输出**：该角色的 `Performance { narrative, dialogue, inner_thoughts }`

#### 编剧 Agent（Editor）
```rust
"你是编剧。收集所有子Agent的表演，合并成连贯的成文：
1. 调 search_vectors 补充必要的远记忆/伏笔
2. 节奏把控、视角切换、过渡衔接
3. 输出最终成文（Markdown）
4. 标注哪些子表演被你裁剪/改动了"
```
**工具集**：`search_vectors`, `get_recent_summary`, `compose`
**输出**：`Draft { text, attribution: [{character_id, kept: bool}] }`

### 3.3 关键设计：专属上下文包（Context Package）

这是整个架构的**灵魂**，也是和 ST 最大的区别。导演 Agent 要为每个子 Agent 构造**不互相污染**的上下文：

```rust
struct ContextPackage {
    character_brief: String,        // 该角色设定（从角色卡提取）
    scene_brief: String,            // 当前场景目标（导演写的）
    relevant_lore: Vec<LoreEntry>,  // 检索到的相关世界书条目（绿灯经向量检索）
    constant_lore: Vec<LoreEntry>,  // 蓝灯常驻条目（所有人共享，但各取所需）
    relevant_memories: Vec<Memory>, // 向量召回的相关远记忆
    recent_window: Vec<Message>,    // 最近 N 条原文（共享窗口）
    task: String,                   // 导演分配的具体任务
}
```

**构造算法**（导演 Agent 用工具调用完成）：
1. 对每个目标角色，用 `(角色名 + 任务关键词)` 检索向量库 → `relevant_memories`
2. 对每个目标角色，匹配世界书绿灯关键词 → `relevant_lore`
3. 蓝灯条目全局共享 → `constant_lore`
4. 最近窗口全局共享 → `recent_window`

### 3.4 并发与取消模型

```rust
// 借鉴 TT 的 DELEGATION_MAX_CONCURRENT_INVOCATIONS=3 设计
struct Orchestrator {
    director: AgentHandle,
    editor: AgentHandle,
    subagents: FuturesUnordered<AgentHandle>,  // 并发池
    cancel_token: CancellationToken,            // 全局取消
}

// 子 Agent 并发上限（移动端省电/省 token）
const MAX_CONCURRENT_SUBAGENTS: usize = 4;

// 每个子 Agent 独立取消（用户点"这个角色我不要了"）
subagent.cancel().await;
```

### 3.5 模型分层策略（成本控制）

| Agent | 推荐模型 | 理由 |
|-------|----------|------|
| 导演 | 强模型（Claude/GPT-4 级） | 要规划、检索、决策 |
| 子 Agent ×N | 廉价/快模型（Gemini Flash / GPT-4o-mini） | 单角色表演，量大 |
| 编剧 | 强模型 | 合并润色需要文学能力 |
| Meta Agent | 中等模型 | 调试分析 |

**用户可在高玩模式为每个 Agent 角色单独配模型**（D11 预设套用）。

### 3.6 提示词范式参考（基于「双人成行 V6.1」预设）

> 用户提供的真实预设（235 条 prompt）揭示了实际写作所需的完整能力图谱。
> 我们**不复刻那份提示词拼接范式**（那是 ST 的提示词注入），而是**把那些能力映射成 Agent 的模块化挂载项**——从"静态拼进 prompt"变成"动态配置 Agent 行为"。

#### 3.6.1 预设能力 → Agent 配置项的映射

| 预设能力域（双人成行编号） | ST 范式 | 我们的 Agent 范式 | 挂载到哪个 Agent |
|------------------------------|---------|-------------------|------------------|
| **COT 推剧情/NPC引入/世界书增强**（#139-144） | 提示词让模型思考 | **真正的工具调用**：search_world_info / emit_plan | 导演 Agent |
| **抗过拟合/抗绝望/抗抢话**（#8-10, 150） | 静态约束文本 | **质量约束模块**（可选挂载） | 编剧 Agent |
| **文风库**（#64-98，数十种） | 选中哪条拼哪条 | **文风模板选择器**（预设套用 D11） | 编剧 Agent |
| **人称视角**（#100-107） | 提示词指定 | **视角参数** | 编剧 Agent |
| **杀八股**（#174-186） | 负面约束文本 | **负面约束模块** | 编剧 Agent |
| **字数/双语对白/内心独白**（#26, 25, 115） | 提示词要求 | **输出规范参数** | 编剧 + 子 Agent |
| **变量更新/日期卡片**（#166-167） | 依赖外部脚本 | **插件能力**（§8） | 插件运行时 |
| **摘要/伏笔**（#169-170） | 依赖外部脚本 | **记忆系统原生**（§7） | 导演 + 编剧工具 |
| **思维链（模型适配）**（#196-220） | 每模型一套 | **按所选模型自动挂载对应 CoT 模块** | 全部 Agent |

#### 3.6.2 三个 Agent 的提示词结构（参考预设分层）

借鉴预设的"分块 + 启用开关"思想，每个 Agent 的系统提示词由**模块组合**而成，而非一整坨：

```rust
struct AgentPromptAssembly {
    // 1. 角色定位（固定，按 AgentRole）
    role_directive: String,
    
    // 2. 模型适配 CoT（按所选模型自动挂）
    cot_module: Option<CotModule>,  // Gemini/Claude/GLM 各一套
    
    // 3. 文风模板（用户选，预设套用 D11）
    style_template: Option<StyleTemplate>,
    
    // 4. 视角参数
    perspective: Perspective,  // 第一/二/三人称/群像
    
    // 5. 质量约束模块（可开关，对应预设的抗性项）
    quality_modules: Vec<QualityModule>,  // 抗抢话/抗绝望/杀八股...
    
    // 6. 输出规范
    output_spec: OutputSpec,  // 字数/双语/内心独白...
    
    // 7. 工具说明（自动生成，按该 Agent 的 tools 清单）
    tool_directives: String,
}
```

#### 3.6.3 导演 Agent 提示词骨架（融合预设 COT 思想）

```
你是写作导演。用户给你写作意图，你要：

【思考阶段】（对应预设 COT 推剧情/NPC引入/世界书增强）
1. 调 search_world_info / search_vectors 了解可用素材和伏笔
2. 判断本场戏：核心冲突是什么？推进哪条剧情线？引入哪些 NPC？
3. 检查：有没有重复（防重复）？有没有违背已建立的设定？

【规划阶段】
4. 决定出场角色，为每个角色分配任务
5. 为每个角色构造专属上下文包（只含它需要的设定，不污染）
6. 输出结构化 Plan

【约束】
- 不要自己写正文
- {{quality_modules}}  ← 这里插入用户选的抗性模块
- {{cot_module}}        ← 这里插入模型适配的思维链指引
```

#### 3.6.4 编剧 Agent 提词词骨架（融合文风/视角/杀八股）

```
你是编剧。收集所有子 Agent 的表演，合并成连贯成文。

【合并】
1. 调 search_vectors 补充必要的远记忆/伏笔
2. 节奏把控、视角切换、过渡衔接

【文风】
{{style_template}}      ← 用户选的文风（轻小说/白描/网文/古风...）

【视角】
{{perspective}}         ← 第一/二/三人称/群像

【质量约束】
{{quality_modules}}     ← 杀八股/抗抢话/抗绝望...

【输出规范】
{{output_spec}}         ← 字数/双语对白/内心独白...

【输出】
最终成文（Markdown），标注哪些子表演被裁剪/改动
```

#### 3.6.5 关键设计决策：模块库可扩展

预设里几十种文风/抗性/视角，我们**不内置全部**，而是：
- 内置 5-8 个**核心模块**（覆盖 80% 场景）
- 提供**模块编辑器**（高玩模式）：用户可自建文风/约束模块
- **支持从 ST 预设导入模块**：解析双人成行那种预设，把每条 prompt 转成一个可挂载模块

这样既保证简洁（普通用户只看到几个核心选项），又给高玩无限扩展空间（D19 双视图）。

#### 3.6.6 三层预设体系（D24 完整设计）

提示词配置是三层结构，从下往上组合：

```
┌─────────────────────────────────────────────────────┐
│  Layer ③  Agent 绑定（运行时生效配置）              │
│  "导演 Agent 当前用：小说预设v2 + DeepSeek 连接"    │
└────────────────────▲────────────────────────────────┘
                     │ 绑定/切换
┌────────────────────┴────────────────────────────────┐
│  Layer ②  提示词预设 Profile（可保存命名）           │
│  "小说预设v2" = {                                    │
│    director: [第一人称, Gemini-CoT, 抗抢话],         │
│    editor:   [白描文风, 杀八股, 字数2000],           │
│    subagent: [角色扮演基准, 内心独白]                │
│  }                                                   │
└────────────────────▲────────────────────────────────┘
                     │ 组合
┌────────────────────┴────────────────────────────────┐
│  Layer ①  提示词模块 Module（最小单位，分类管理）   │
│  ┌──────────┬──────────┬──────────┬──────────┐      │
│  │ 视角组   │ CoT组    │ 文风组   │ 约束组   │      │
│  │ 第一人称 │ Gemini   │ 白描     │ 杀八股   │      │
│  │ 第二人称 │ Claude   │ 轻小说   │ 抗抢话   │      │
│  │ 第三人称 │ GLM      │ 网文     │ 抗绝望   │      │
│  │ 群像     │ DS       │ 古风     │ 防重复   │      │
│  └──────────┴──────────┴──────────┴──────────┘      │
│  每组内**互斥单选**（选了第一人称就不能同时第二人称）│
└─────────────────────────────────────────────────────┘
```

**模块分类与互斥规则**：

| 组（category） | 互斥性 | 示例模块 |
|----------------|--------|----------|
| `perspective` 视角 | 单选 | 第一/二/三人称、群像、双视角 |
| `cot` 思维链 | 单选（按模型） | Gemini-CoT / Claude-CoT / GLM-CoT / DS-CoT / 自由CoT |
| `style` 文风 | 单选 | 白描/轻小说/网文/古风/ASMR/魔幻现实... |
| `quality` 质量约束 | **多选** | 杀八股/抗抢话/抗绝望/防重复/反神化... |
| `output` 输出规范 | 多选 | 字数控制/双语对白/内心独白/格式要求 |
| `tone` 情感基调 | 单选 | 治愈/伤感/积极/消极/自定义 |

**切换语义**（D27 按钮）：
- 单选组：点新按钮 = 替换（旧的取消）
- 多选组：点按钮 = 切换开/关
- 切换**即时生效**（下次 Agent 调用就用新配置），无需重启

#### 3.6.7 提示词组装流程（运行时）

每次 Agent 调用 LLM 前，后端按绑定关系组装系统提示词：

```rust
fn assemble_system_prompt(agent: &AgentRole, profile: &PromptProfile) -> String {
    let mut parts = vec![
        base_role_directive(agent),           // 角色定位（固定）
    ];
    
    // 按组的优先级顺序，插入已选模块
    for category in [Perspective, Cot, Style, Quality, Output, Tone] {
        if let Some(module) = profile.selected_for(agent, category) {
            parts.push(module.content.clone());
        }
    }
    
    parts.push(tool_directives(agent));        // 工具说明（自动生成）
    
    parts.join("\n\n---\n\n")
}
```

#### 3.6.8 预设的导入/导出/分享

- **导出**：当前 Profile → JSON 文件（含所用模块的引用 + 自定义内容）
- **导入**：JSON → 新 Profile（模块按 ID 匹配，缺失的自动建为自定义模块）
- **从 ST 预设导入**：见 §9.4（Meta Agent 自动整理）

### 3.7 对话管理（分支/重 roll/编辑/删除，D28）

写作离不开对生成结果的反复打磨。本节定义对话树结构及核心操作。

#### 3.7.1 数据结构：对话树（非扁平列表）

我们的对话**不是** ST 那种扁平数组，而是**树**——每条消息可有多个版本（分支），每个版本是 AI 的一次完整产出：

```rust
struct Conversation {
    id: ConversationId,
    character_id: CharacterId,
    root: MessageNodeId,           // 首条消息
}

struct MessageNode {
    id: NodeId,
    parent_id: Option<NodeId>,     // 父消息（首条为 None）
    variants: Vec<MessageVariant>, // 同一位置的多个版本（分支/swipe）
    active_variant: usize,         // 当前选中的版本索引
}

struct MessageVariant {
    id: VariantId,
    role: Role,                    // User / Assistant
    content: String,
    created_at: DateTime,
    status: VariantStatus,         // Draft / Final / Discarded
    // 若是 Agent 产出，保留溯源信息（便于部分重 roll）
    provenance: Option<Provenance>,
}

struct Provenance {
    session_id: SessionId,         // 来自哪次写作流水线
    plan: Option<PlanSnapshot>,    // 当时的 Plan 快照
    subagent_results: Vec<PerformanceSnapshot>, // 各子 Agent 产出快照
    profile_id: ProfileId,         // 当时用的提示词预设
    seed: u64,                     // 随机种子（重 roll 时可换）
}

enum VariantStatus {
    Draft,       // 编辑中/未定稿
    Final,       // 已采纳（才会触发记忆归档）
    Discarded,   // 被重 roll 或手动丢弃（不归档，但保留可恢复）
}
```

**关键设计**：
- **Discarded 不删除**，软删除保留（用户可翻历史找回被丢弃的版本）
- **只有 Final 才进记忆归档**（D28，防止垃圾总结污染向量库）
- 每个变体带 `Provenance`——这是部分重 roll 的基础

#### 3.7.2 核心操作

| 操作 | 行为 | 对应 ST |
|------|------|---------|
| **编辑消息** | 改当前 variant 的 content；保留编辑历史（可撤销） | ✅ 同 |
| **删除消息** | 软删除当前 variant（→ Discarded）；若该 node 还有其他 variant 则切到下一个，否则 node 标记为空 | ST 是硬删，我们是软删可恢复 |
| **分支 swipe** | 同一 node 新增一个 variant，旧的保留 | ✅ 同 |
| **切换版本** | 改 `active_variant`，渲染对应内容 | ✅ 同（左右滑） |
| **继续生成** | 编剧 Agent 以当前 variant 为前文，续写 | ✅ 同 |
| **整体重 roll** | 整条流水线重跑（导演→子×N→编剧），新 variant | ✅ 同 ST 的 roll |
| **部分重 roll** ⭐ | 见 3.7.3，我们独有 | ❌ ST 做不到 |

#### 3.7.3 部分重 roll（独有能力，重点设计）

利用 `Provenance` 里保存的各子 Agent 产出快照，可以**只重跑不满意的部分**：

```
用户对当前成文不满意："角色 B 演得太僵硬"
       │
       ▼
选择【部分重 roll: 子 Agent B】
       │
       ▼
后端：
  1. 读 Provenance.subagent_results，取出 A、C 的产出（保留）
  2. 只重跑子 Agent B（可换种子/换参数/换连接）
  3. 把新 B + 旧 A + 旧 C 喂给编剧 Agent 重新合并
  4. 产出新 variant，作为同 node 的新分支
       │
       ▼
用户对比新旧版本，满意则采纳
```

**成本对比**（假设 3 个子 Agent）：
- 整体重 roll：导演 + 3 子 + 编剧 = **5 次** LLM 调用
- 部分重 roll（只重 B）：1 子 + 编剧 = **2 次** LLM 调用（省 60%）

**支持的子粒度**：
- 只重跑某 1 个子 Agent（最常用）
- 只重跑编剧（子产出都满意，但合并得不好）
- 只重跑导演（想换规划思路，但子 Agent 跟随新 Plan）

**不支持**的部分重 roll：
- 不能只重跑"导演"却保留"旧子产出"——因为子产出依赖 Plan，Plan 变了旧子产出就不匹配了（后端会校验，拒绝这种组合）

#### 3.7.4 重 roll 与记忆系统的关系

```
写作流水线产出 variant (Draft)
       │
       ├─ 用户不满意 → 重 roll → 新 variant（旧的转 Discarded，不归档）
       │
       └─ 用户满意 → 采纳为 Final
                        │
                        ▼
                   触发记忆归档检查
                   （§7 的 maybe_archive：超阈值才归档）
```

**被重 roll 的旧版本不归档**——它们是创作过程的脚手架，不是正式剧情。这避免向量库被" discarded 的废稿"污染。

#### 3.7.5 前端交互（移动端）

```
┌─────────────────────────────────────┐
│  📝 [第3条] 用户：写一场雨中告别     │
│                                     │
│  ✨ [第4条] AI（v2/3）  ◀ 2/3 ▶    │  ← 版本切换条
│  ─────────────────────────────────  │
│  雨滴敲在屋檐……（当前版本内容）     │
│  ─────────────────────────────────  │
│  [✏️编辑] [🔄重roll▼] [🗑删除]      │
│             ├ 整体重roll            │
│             ├ 只重跑 编剧           │
│             └ 只重跑 子AgentB  ⭐   │
└─────────────────────────────────────┘
```

- 版本切换条 `◀ 2/3 ▶`：左右滑或点箭头切 variant
- 重 roll 按钮展开**菜单**，让用户选粒度（这是 ST 没有的）
- 长按消息：弹出更多操作（复制/书签/继续生成/导出此版本）

---

## 4. 资料源体系

### 4.1 角色卡（ST V2/V3 兼容导入）

**导入格式**：
- PNG（embed JSON in `tEXt` chunk）
- 纯 JSON（V2/V3 schema）
- 多卡打包（ZIP）

**Rust 解析**：
```rust
mod character_import {
    // PNG embed 提取（参考 ST 的 png 模块）
    pub fn extract_from_png(bytes: &[u8]) -> Result<CharacterCard>;
    // V2/V3 JSON schema 反序列化
    pub fn parse_v3(json: Value) -> Result<CharacterCard>;
    // 内嵌 character_book → 世界书
    pub fn extract_embedded_world_info(card: &CharacterCard) -> Option<WorldInfoBook>;
    // extensions.character_assets / HTML 代码 → 标记为"可渲染"
    pub fn extract_renderable_assets(card: &CharacterCard) -> RenderableAssets;
}
```

**导入后的内部表示**（不保留 ST 的扁平结构，重组为领域模型）。

### 4.2 世界书（蓝灯/绿灯/可调路由 D13）

```rust
enum LoreRoute {
    Constant,    // 蓝灯：进导演常驻上下文
    Selective,   // 绿灯：进向量检索池（默认）
    Both,        // 两者都走
    Disabled,    // 不使用
}

struct WorldInfoEntry {
    id: EntryId,
    keys: Vec<String>,           // 触发关键词
    content: String,
    constant: bool,              // 原始蓝绿灯
    route: LoreRoute,            // 用户可调（D13），默认按 constant 映射
    order: i32,
    position: InjectionPosition, // ST 原值，我们大多忽略（不注入）
    // ...
}

impl WorldInfoEntry {
    fn default_route(&self) -> LoreRoute {
        if self.constant { LoreRoute::Constant } else { LoreRoute::Selective }
    }
}
```

⚠️ **范式说明**（写进用户文档）：我们**不把世界书注入 prompt**。蓝灯进导演常驻，绿灯进向量池被 Agent 主动检索。

### 4.3 预设（一键套用 D11 + 正则载体 D9/D12）

```rust
struct Preset {
    id: PresetId,
    name: String,
    // 提示词模板（可套用到 Agent）
    system_prompt_template: String,  // 含 {{char}} {{user}} 等占位符
    // spreset 式正则脚本
    regex_scripts: Vec<RegexScript>,
    // 元信息
    source: PresetSource,  // ImportedFromST / Native
}

struct RegexScript {
    name: String,
    find_regex: String,
    replace_string: String,
    placement: RegexPlacement,  // Input(用户→导演前) / Output(编剧成文后)
    disabled: bool,
}

// 套用到某个 Agent 角色（D11）
fn apply_preset_to_agent(preset: &Preset, role: AgentRole) -> AppliedPreset {
    // 把 system_prompt_template 填充占位符后，设为该 Agent 的系统提示词
    // 把 regex_scripts 注册到对应作用域
}
```

**正则作用域**（D12，重申）：
- `RegexPlacement::Input` → 在用户输入到达**导演 Agent** 之前应用
- `RegexPlacement::Output` → 在**编剧 Agent** 输出成文之后应用
- 子 Agent 内部**不跑正则**

---

## 5. Rust 数据模型（核心 struct）

> 完整定义在 `domain/models/`，这里给关键骨架。

```rust
// ============ 角色/世界书/预设（见 §4） ============

// ============ 记忆系统 ============
struct MemoryStore {
    recent_window: VecDeque<Message>,       // 滑动窗口（默认 50）
    archived_summaries: Vec<ArchivedSummary>, // 远记忆大总结（shujuku 借鉴）
    vector_index: VectorIndex,              // 向量索引（见 §7）
}

struct ArchivedSummary {
    id: SummaryId,
    content: String,           // ≤500TK 高密度总结
    source_message_range: (usize, usize), // 归档自哪些原文
    created_at: DateTime,
    vector: Option<Vec<f32>>,  // 嵌入向量
    keywords: Vec<String>,     // 关键词索引
}

// ============ Agent ============
struct AgentProfile {
    id: ProfileId,
    role: AgentRole,           // Director / Subagent / Editor / Meta
    system_prompt: String,
    model_ref: ModelRef,
    max_tool_rounds: usize,
    tools: Vec<ToolSpec>,
    // 委派能力（仅 Director 有）
    delegation: Option<DelegationConfig>,
}

enum AgentRole { Director, Subagent(CharacterId), Editor, Meta }

struct Plan {
    scene_brief: String,
    subagent_tasks: Vec<SubagentTask>,
}

struct SubagentTask {
    character_id: CharacterId,
    brief: String,
    context_package: ContextPackage,
}

// ============ 运行时状态 ============
enum PipelineState {
    Idle, Directing, Delegating, Editing, Review, Committed, Aborted,
}

struct WritingSession {
    id: SessionId,
    intent: String,
    state: PipelineState,
    plan: Option<Plan>,
    subagent_results: Vec<Performance>,
    draft: Option<Draft>,
    events: Vec<PipelineEvent>,   // 流式事件历史（前端订阅）
}

// ============ 提示词模块/预设体系（D24）============
use prompt_module::{ModuleCategory, Exclusivity};

struct PromptModule {
    id: ModuleId,
    name: String,                  // "第一人称" / "Gemini-CoT" / "白描"
    category: ModuleCategory,      // Perspective / Cot / Style / Quality / Output / Tone
    content: String,               // 模块正文（拼进 system prompt 的文本）
    exclusivity: Exclusivity,      // Single(单选互斥) / Multiple(多选叠加)
    source: ModuleSource,          // BuiltIn / ImportedFromST / UserCustom
    applicable_roles: Vec<AgentRole>, // 该模块可挂到哪些 Agent（如 CoT 全部，文风仅 Editor）
    metadata: ModuleMeta,          // 作者/版本/标签（便于搜索和分享）
}

// 一组同 category 的模块互斥（如 perspective 组内只能选一个）
// 不同 category 的模块叠加

struct PromptProfile {
    id: ProfileId,
    name: String,                  // "我的小说预设 v2"
    // 每个 Agent 角色选定哪些模块（按 category 组织）
    selections: HashMap<AgentRole, HashMap<ModuleCategory, Vec<ModuleId>>>,
    // 自定义覆盖（用户直接改某 Agent 的提示词，不走模块）
    overrides: HashMap<AgentRole, Option<String>>,
    created_at: DateTime,
    source: ProfileSource,         // UserCreated / ImportedFromST / BuiltIn
}

// 运行时：当前每个 Agent 绑定的 Profile + 连接（D24/D25）
struct AgentBinding {
    role: AgentRole,
    active_profile_id: ProfileId,
    active_connection_id: ConnectionId,
}

// ============ LLM 连接配置（D25）============
struct LlmConnection {
    id: ConnectionId,
    name: String,                  // "DeepSeek 官方" / "我的 Gemini"
    base_url: String,              // https://api.deepseek.com/v1
    api_key_ref: SecretRef,        // 指向 Keystore 的引用，不明文存
    model: String,                 // deepseek-chat / gemini-2.5-pro
    // 兼容协议（决定请求格式）
    protocol: LlmProtocol,         // OpenAI / Anthropic / Gemini / Custom
    // 采样参数（可被 Agent 角色覆盖）
    default_params: SamplingParams,
    source: ConnectionSource,      // Template(预填) / UserCustom
}

struct SamplingParams {
    temperature: Option<f32>,
    top_p: Option<f32>,
    max_tokens: Option<usize>,
    // ... 可扩展
}

enum LlmProtocol { OpenAi, Anthropic, Gemini, Custom(String) }

// 预填的连接模板（用户选"DeepSeek"就自动填好 URL/协议，只需填 key）
struct ConnectionTemplate {
    name: String,                  // "DeepSeek" / "Gemini" / "OpenAI" / "SiliconFlow"
    base_url: String,
    protocol: LlmProtocol,
    default_model: String,
    models: Vec<String>,           // 可选模型列表
    get_key_hint: String,          // "到 platform.deepseek.com 申请"
}

// ============ 安全：密钥存储 ============
// api_key 不存进 settings.json，走 Android Keystore
struct SecretRef(String);  // 形如 "keystore:llm_conn_<id>"
// 后端通过 infra-secrets crate 读写，settings 只存引用

// ============ 插件 ============
struct PluginManifest {
    id: String,
    name: String,
    version: String,
    permissions: Vec<Permission>,    // 声明式权限
    entry_html: String,              // 渲染入口
    ui_slots: Vec<UiSlot>,           // 要挂载的 UI 位置
    event_subscriptions: Vec<String>,// 订阅的事件
}

enum Permission {
    ReadCharacters, ReadWorldInfo, ReadMemory,
    WriteVariables, CallLlm, Network, Notifications,
}

enum UiSlot {
    MessageDecorator,    // 消息装饰
    SidebarPanel,        // 侧栏面板
    MetaAgentToolbar,    // Meta Agent 工具栏
    ComposerAddon,       // 输入框扩展
}
```

---

## 6. 模块拆分（crate 级）

采用**单一 workspace + 多 crate**（比 TT 的单 crate 巨石更清晰）：

```
storyforge/
├── Cargo.toml                 # workspace
├── crates/
│   ├── domain/                # 纯领域模型，无 IO，无 tauri 依赖
│   │   ├── character.rs
│   │   ├── world_info.rs
│   │   ├── preset.rs
│   │   ├── memory.rs
│   │   ├── agent.rs
│   │   └── plugin.rs
│   │
│   ├── infra-storage/         # 文件持久化（JSON）
│   ├── infra-llm/             # LLM HTTP client 池 + 流式
│   ├── infra-vector/          # 向量库（本地）+ 嵌入 API client
│   ├── infra-import/          # ST 卡/世界书/预设导入解析
│   ├── infra-regex/           # 正则引擎（regress crate，纯 Rust）
│   ├── infra-plugin-host/     # 插件沙箱宿主（在 frontend 侧，但 API 在这定义）
│   │
│   ├── app-memory/            # 记忆系统应用服务（归档/召回/rerank）
│   ├── app-agent/             # Agent 运行时（委派/工具/取消）
│   ├── app-pipeline/          # 写作流水线编排（依赖 app-agent, app-memory）
│   ├── app-conversation/      # 对话管理（对话树/分支/重roll/编辑/删除，§3.7）
│   ├── app-logging/           # 日志收集中枢（后端/LLM/前端三类，§12）
│   ├── app-meta/              # Meta Agent（诊断/patch/配置 Agent）
│   │
│   └── tauri-app/             # Tauri v2 入口 + commands 注册
│       ├── src/
│       │   ├── main.rs
│       │   ├── commands/      # 所有 #[tauri::command]
│       │   └── lib.rs
│       └── src-tauri/         # Android target 配置
│
├── frontend/                  # 前端（选型见 §10）
│   ├── src/
│   └── ...
│
└── docs/
    ├── INTENT.md
    ├── TECHNICAL_DESIGN.md    # 本文件
    └── AGENT_DESIGN.md        # Agent 提示词/工具详细文档（后续）
```

**依赖方向**（严格单向，禁止环）：
```
tauri-app → app-* → domain
                 ↘ infra-* → domain
```

**为什么不直接 fork TT**：
- TT 是单 crate 12 万行巨石，含大量我们不要的（iOS/LAN/tray/...)
- TT 的数据模型贴合 ST（保留兼容），我们要 Agent 优先
- 多 crate 更利于测试和长期维护

**借鉴 TT 的具体点**（读代码学思路，不抄代码）：
- Agent 委派机制（`DELEGATION_MAX_CONCURRENT_INVOCATIONS`）
- Agent profile schema 设计
- Tauri command 分组注册模式
- LLM HTTP client pool + 流式 channel

---

## 7. 记忆系统原生实现（参考 shujuku）

> 这是把 shujuku 的成熟工作流用 Rust 原生重写，**算法借鉴，调用方式改变**（从"自动注入"变成"Agent 工具调用"）。

### 7.1 三层记忆架构

```
┌─────────────────────────────────────────────┐
│  Layer 1: Recent Window（近期原文）          │
│  · 最近 N 条消息原文（默认 50）              │
│  · 滑动窗口，FIFO                            │
│  · 直接进所有 Agent 的上下文                 │
└──────────────────┬──────────────────────────┘
                   │ 窗口溢出时触发归档
                   ▼
┌─────────────────────────────────────────────┐
│  Layer 2: Archived Summary（远记忆大总结）   │
│  · shujuku 工作流：批量纪要 → LLM 压缩       │
│  · ≤500TK，高密度（人物/事件/目标/冲突/伏笔）│
│  · 并发归档（tokio，archiveMaxConcurrency=3）│
│  · 归档后从近期窗口移除原文                  │
└──────────────────┬──────────────────────────┘
                   │ 每条总结嵌入向量化
                   ▼
┌─────────────────────────────────────────────┐
│  Layer 3: Vector Index（向量检索池）         │
│  · 远记忆总结 + 绿灯世界书条目 + 角色设定片段│
│  · 嵌入走远程 API（D14）                     │
│  · 检索：关键词生成 → 向量召回 → rerank      │
└─────────────────────────────────────────────┘
```

### 7.2 归档工作流（Rust 实现）

```rust
// app-memory/src/archiver.rs
pub struct MemoryArchiver {
    llm: Arc<LlmClient>,
    embedder: Arc<Embedder>,
    vector_store: Arc<VectorStore>,
    config: ArchiveConfig,
}

pub struct ArchiveConfig {
    pub threshold: usize,              // 50：触发阈值
    pub archive_batch_size: usize,     // 3
    pub archive_trigger_count: usize,  // 9  → 单次归档 27 条
    pub max_concurrency: usize,        // 3
    pub summary_max_tokens: usize,     // 500
}

impl MemoryArchiver {
    pub async fn maybe_archive(&self, store: &mut MemoryStore) -> Result<()> {
        if store.recent_window.len() < self.config.threshold { return Ok(()); }
        
        // 取出待归档的窗口尾部
        let to_archive: Vec<_> = store.recent_window.drain(..batch_size * trigger_count).collect();
        
        // 分批并发归档（借鉴 shujuku summaryPromptGroup）
        let batches = to_archive.chunks(self.config.archive_trigger_count);
        let summaries = futures::stream::iter(batches)
            .map(|batch| self.archive_batch(batch))
            .buffer_unordered(self.config.max_concurrency)
            .try_collect::<Vec<_>>()
            .await?;
        
        // 每条总结：嵌入 → 入向量库
        for summary in summaries {
            let vector = self.embedder.embed(&summary.content).await?;
            self.vector_store.upsert(VectorRecord {
                id: summary.id,
                content: summary.content.clone(),
                vector,
                keywords: summary.keywords,
                kind: VectorKind::ArchivedSummary,
            }).await?;
            store.archived_summaries.push(summary);
        }
        Ok(())
    }
    
    async fn archive_batch(&self, messages: &[Message]) -> Result<ArchivedSummary> {
        // 直接复用 shujuku 提炼的 prompt（高密度总结，500TK，优先级：人物/事件/目标/冲突/道具/伏笔）
        let prompt = self.config.summary_prompt_group.render(messages);
        let content = self.llm.generate(&prompt).await?;
        Ok(ArchivedSummary { content, /* ... */ })
    }
}
```

**总结 prompt 模板**（直接复用 shujuku 的精炼版本，已验证有效）：
```
你负责将一批较早的对话整理为可供长期召回的远记忆大总结。
目标：生成一条可被向量召回使用的高密度长期记忆。
硬性长度约束：最终输出最高 500TK；信息过多优先压缩，不要扩写。
内容优先级：人物关系、关键事件、目标变化、冲突、重要道具、地点、时间线、未解决伏笔。
输出要求：只输出最终远记忆大总结正文。
```

### 7.3 召回工作流（关键词 + 向量 + rerank）

```rust
// app-memory/src/recall.rs
pub struct MemoryRecaller {
    llm: Arc<LlmClient>,
    embedder: Arc<Embedder>,
    vector_store: Arc<VectorStore>,
    reranker: Option<Arc<Reranker>>,  // 可选
}

impl MemoryRecaller {
    /// Agent 工具调用入口（替代 shujuku 的自动注入）
    pub async fn recall(&self, query: &str, top_k: usize) -> Result<Vec<MemoryHit>> {
        // Step 1: LLM 生成检索关键词（借鉴 shujuku keywordPromptGroup）
        let keywords = self.generate_keywords(query).await?;
        
        // Step 2: 关键词过滤候选
        let candidates = self.vector_store.search_by_keywords(&keywords, 1000).await?;
        
        // Step 3: 向量相似度召回
        let qvec = self.embedder.embed(query).await?;
        let mut hits = self.vector_store.search_by_vector(&qvec, top_k).await?;
        
        // Step 4: 合并候选，rerank（可选）
        if let Some(reranker) = &self.reranker {
            hits = reranker.rerank(query, &hits).await?;
        }
        
        // Step 5: minScore 过滤
        Ok(hits.into_iter().filter(|h| h.score >= 0.45).collect())
    }
}
```

### 7.4 向量库选型（移动端约束）

**需求**：纯本地、嵌入式、Android 可跑、支持 ANN（近似最近邻）。

| 选项 | 优点 | 缺点 | 决策 |
|------|------|------|------|
| **自实现（IVF/HNSW）** | 无外部依赖，体积小 | 要自己写 ANN | M0 先暴力检索，M2 上 HNSW |
| `hnsw_rs` crate | 纯 Rust，HNSW 成熟 | 内存索引，需持久化层 | ✅ **M2 采用** |
| SQLite + `sqlite-vss` | 持久化好 | 要编译扩展，Android NDK 麻烦 | ❌ 放弃 |
| LanceDB | 功能全 | 包体大，Android 支持弱 | ❌ 放弃 |

**决策**：M0 用 `Vec<VectorRecord>` + 暴力余弦相似度（数据量小时够用）；M2 引入 `hnsw_rs` 做 ANN。持久化用 JSON/MessagePack（与 ST 数据同构，便于导出）。

### 7.5 嵌入模型（远程 API，D14）

```rust
pub struct Embedder {
    client: reqwest::Client,
    config: EmbedConfig,
}

pub struct EmbedConfig {
    pub endpoint: String,    // 如 OpenAI /v1/embeddings 或 SiliconFlow
    pub api_key: Secret<String>,
    pub model: String,       // text-embedding-3-small / bge-large-zh
    pub dim: usize,
}
```
用户在高玩模式配置。**默认推荐 SiliconFlow 的 bge-large-zh**（中文效果好、便宜）。

---

## 8. 插件运行时设计

### 8.1 架构

```
┌──────────────────────────────────────────────────┐
│  前端 webview                                     │
│  ┌────────────────────────────────────────────┐  │
│  │  插件 iframe 沙箱（每个插件一个）           │  │
│  │  · sandbox="allow-scripts"                 │  │
│  │  · CSP 严格                                │  │
│  │  · srcdoc = 插件 entry_html                │  │
│  │  ┌──────────────────────────────────────┐  │  │
│  │  │  window.storyforge API（注入的桥）   │  │  │
│  │  │  · 读角色/世界书/记忆                │  │  │
│  │  │  · 读写变量                          │  │  │
│  │  │  · 订阅事件                          │  │  │
│  │  │  · 挂载 UI 到 slot                   │  │  │
│  │  └──────────────────────────────────────┘  │  │
│  └──────────────────┬─────────────────────────┘  │
│                     │ postMessage                  │
│  ┌──────────────────▼─────────────────────────┐  │
│  │  Plugin Host（前端侧 JS）                  │  │
│  │  · 权限校验（按 manifest）                 │  │  │
│  │  · 转发到后端                              │  │
│  └──────────────────┬─────────────────────────┘  │
└─────────────────────┼────────────────────────────┘
                      │ Tauri invoke
┌─────────────────────▼────────────────────────────┐
│  后端 plugin_commands.rs                          │
│  · plugin_read_character / plugin_read_memory ... │
│  · 权限二次校验                                   │
└──────────────────────────────────────────────────┘
```

### 8.2 权限模型（移动端安全命脉）

插件**只能用 manifest 声明的权限**。后端每个 API 二次校验：
```rust
#[tauri::command]
async fn plugin_read_character(
    plugin_id: String,
    character_id: CharacterId,
    state: State<'_, AppState>,
) -> Result<Character, CommandError> {
    let plugin = state.plugin_registry.get(&plugin_id)?;
    plugin.ensure_permission(&Permission::ReadCharacters)?;  // 权限校验
    state.character_service.get(character_id).await
}
```

### 8.3 API 设计（自有，非 ST 兼容）

```js
// 插件作者用的 API（window.storyforge）
window.storyforge = {
  character: {
    getCurrent(): Promise<Character>,
    get(id): Promise<Character>,
  },
  worldInfo: {
    search(query): Promise<LoreEntry[]>,  // 走我们的向量检索
  },
  memory: {
    recall(query, topK): Promise<Memory[]>,
  },
  variables: {
    get(key): Promise<any>,
    set(key, value): Promise<void>,  // 需 WriteVariables 权限
  },
  events: {
    on(event, cb): Subscription,  // pipeline.state_changed / message.finalized ...
  },
  ui: {
    mountToSlot(slot, element): void,
  },
  // 不暴露：callLlm（独立权限）、network（独立权限+白名单）
}
```

### 8.4 插件分发
- 本地安装（用户从文件导入 `.sfplugin.zip`）
- 未来：内置插件商店（不在 MVP）

### 8.5 ST → 我们的 API 映射表（Meta Agent 插件生成依据）

> 这是 Meta Agent 把 ST 插件"翻译"成我们插件的核心依据。
> 来源：分析 ST 扩展 API + 双人成行预设里插件相关 prompt（#165-172, 212）。

| ST 扩展 API | 我们的等价 API | 说明 |
|-------------|----------------|------|
| `SillyTavern.getContext()` | `storyforge.character.getCurrent()` + `storyforge.worldInfo.search()` | 拆分成多个细粒度 API |
| `getContext().characters` | `storyforge.character.list()` / `.get(id)` | |
| `getContext().chat` | `storyforge.memory.getRecent()` | 走记忆系统 |
| `eventTypes.MESSAGE_RECEIVED` | `storyforge.events.on('message.finalized', cb)` | 事件名重定义 |
| `eventTypes.GENERATION_STARTED` | `storyforge.events.on('pipeline.state_changed', cb)` | |
| `setLocalVar(key, val)` | `storyforge.variables.set(key, val)` | 需 WriteVariables 权限 |
| `getLocalVar(key)` | `storyforge.variables.get(key)` | |
| `replaceVariables(msg)` | （不暴露，变量替换由后端统一做） | |
| `triggerSlash('/genraw ...')` | `storyforge.llm.generate(prompt)` | 需 CallLlm 权限 |
| `$('#chat').append(html)` | `storyforge.ui.mountToSlot('message_decorator', el)` | DOM → UI slot |
| `extension_settings[myExt]` | `storyforge.storage.get/set` | 持久化 |
| `saveSettingsDebounced()` | 自动（storage.set 已防抖） | |
| `fetch('/api/characters/...')` | （不暴露网络，统一走 storyforge API） | |
| `registerSlashCommand(...)` | （暂不支持，M5+ 考虑） | |

**无法映射的 ST 能力**（明确告知插件作者）：
- ❌ 直接操作 ST 的 DOM 结构 → 改用 UI slot 挂载
- ❌ 拦截/修改 prompt 注入 → 我们不注入，改用 Agent 工具
- ❌ 访问其他扩展的内部状态 → 沙箱隔离

### 8.6 角色卡 iframe 渲染（D7，与插件运行时共用沙箱）

带前端的角色卡 = 一种特殊插件：
```rust
enum SandboxKind {
    Plugin(PluginManifest),     // 用户安装的插件
    CharacterCard(CardAssets),  // 角色卡内嵌 HTML（来自 ST 导入）
}
// 共用同一套沙箱+API桥，但权限不同：
// 角色卡默认只有 ReadCharacters + WriteVariables
```

---

## 9. Meta Agent 设计

### 9.1 形态（D16：对话框，多轮）

```
┌─────────────────────────────────────┐
│  Meta Agent 对话框                  │
│  ─────────────────────────────────  │
│  用户：帮我看看这个世界书有没有冲突 │
│                                      │
│  Agent：[调用 inspect_world_info]    │
│        发现 2 处可能冲突：           │
│        ① 条目#12 和 #47 关键词重叠   │
│        ② 条目#23 提到的"老王"未在    │
│           任何角色卡中定义           │
│        建议如下修改：                │
│        [查看 patch ▼]  [采纳] [忽略]│
└─────────────────────────────────────┘
```

### 9.2 工具集（诊断型 + patch 型 + 插件生成型）

```rust
// 只读诊断工具（默认全部启用）
meta_inspect_world_info(book_id) -> WorldInfoReport
meta_inspect_preset(preset_id) -> PresetReport
meta_inspect_character_card(card_id) -> CardReport
meta_count_tokens(text) -> usize
meta_find_conflicting_entries() -> Vec<Conflict>
meta_find_orphan_entries() -> Vec<EntryId>  // 绿灯从没被触发
meta_check_renderable_card(card_id) -> JsReport  // iframe 卡的 JS 报错

// patch 工具（提议 → 用户采纳，模式 C）
meta_propose_patch(target, change) -> Patch   // 不直接改
meta_apply_patch(patch_id) -> Result<()>       // 用户点采纳才执行

// === 插件 CRUD（用户补充，v4）===
meta_list_plugins() -> Vec<PluginSummary>          // 查
meta_inspect_plugin(id) -> PluginReport            // 查详情（含代码/权限/挂载槽）
meta_generate_plugin_from_st(st_zip_bytes, intent) -> PluginDraft
  // 增：把 ST 插件"翻译"成我们的 .sfplugin 骨架
  //   · 分析 ST 插件实际行为（读 DOM? 改 prompt? 管变量? 渲染 UI?）
  //   · 映射：ST getContext() → storyforge.character.get()
  //          ST eventSource  → storyforge.events.on()
  //          ST DOM 操作     → storyforge.ui.mountToSlot()
  //   · 生成 manifest + entry_html + 桥代码
  //   · 标注"已转换/无法转换/需手动"的部分
meta_install_plugin_draft(draft_id) -> PluginId     // 增：安装到插件区
meta_edit_plugin_code(id, change) -> Patch          // 改：提议代码改动
meta_delete_plugin(id) -> Result<()>                // 删：卸载
```

### 9.3 插件生成的工作模式（重点说明）

Meta Agent **不是**自动转换器，是**插件开发助手**。流程：

```
用户：[上传 ST 插件 ZIP] 帮我做个等价的
       │
       ▼
Meta Agent（多轮对话）:
  1. 读 ST 插件代码 → 分析"它实际在做什么"
     （输出行为清单：读角色? 写变量? 渲染 UI? 监听事件?）
  2. 映射到我们的能力（API 对照表）
     · ST 的 getContext()        → storyforge.character.get()
     · ST 的 eventSource.on()    → storyforge.events.on()
     · ST 的 DOM 操作            → storyforge.ui.mountToSlot()
     · ST 的 setLocalVar()       → storyforge.variables.set()
  3. 生成 .sfplugin 骨架（manifest + entry_html + 桥代码）
     · 明确标注：✅已自动转换 / ⚠️需手动改 / ❌无法对应
  4. [采纳] → 装到插件区
  5. 进入"调试对话"循环：
     用户运行插件 → 报错 → Meta Agent 提议修复 → [采纳] → 再运行
  6. 用户满意后，插件固化
```

**安全约束**：
- 生成的插件仍受**权限模型**约束（§8.2），不能越权
- 复杂插件（如 shujuku 215KB）一次生成不现实，Meta Agent 会**分模块**逐步生成
- Meta Agent **不会**自动执行生成的代码，必须用户手动运行测试

### 9.4 配置 Agent 的工作流（D26 重点）

Meta Agent 的第二大核心能力：**读 ST 预设 → 自动整理成模块库 → 提议分配给各 Agent → 用户采纳**。
这解决了"我有几百条 ST prompt，怎么搬过来"的迁移痛点。

#### 9.4.1 ST 预设导入工作流

```
用户：[上传 双人成行 V6.1.json] 帮我整理配置到各 Agent
       │
       ▼
Meta Agent（多轮对话）:

  ① 解析：读出 235 条 prompt + prompt_order（启用顺序）
       │
       ▼
  ② 分类：把每条 prompt 归入我们的 6 大模块组
     · #101-107 人称  → perspective 组（7 条 → 5 个视角模块）
     · #196-220 CoT   → cot 组（按 Gemini/Claude/GLM 分组）
     · #64-98 文风    → style 组（数十种文风）
     · #174-186 杀八股→ quality 组（多选约束）
     · #26,115 字数等 → output 组
     · 无法归类的      → 标记"待人工确认"
       │
       ▼
  ③ 提议分配：按 prompt 的 role 和位置，建议挂到哪个 Agent
     · 导演类（推剧情/NPC引入/世界书增强）→ Director
     · 文风/视角/输出规范             → Editor
     · 角色扮演基准/内心独白           → Subagent
       │
       ▼
  ④ 生成 Patch：新建一批 PromptModule + 一个 PromptProfile
     · 模块 source 标记为 ImportedFromST（保留来源）
     · Profile 命名为 "双人成行 V6.1（导入）"
       │
       ▼
  ⑤ 用户预览：
     "整理出 47 个模块，归类如下：
      · 视角 5 个 / CoT 4 套 / 文风 23 种 / 约束 12 个 / 输出 3 个
      · 建议导演用[Gemini-CoT+推剧情]，编剧用[白描+杀八股]
      · 有 6 条无法自动归类，需你确认：[列表]"
     [查看模块清单] [查看分配建议] [全部采纳] [逐项确认]
       │
       ▼
  ⑥ 采纳 → 模块入库 + Profile 生效 → 用户可在 Agent 卡片一键切换
```

#### 9.4.2 配置 Agent 的工具集

```rust
// === 提示词模块/预设管理 ===
meta_list_prompt_modules(filter) -> Vec<PromptModule>       // 查
meta_create_prompt_module(module) -> ModuleId               // 增（用户/Meta Agent 自建）
meta_update_prompt_module(id, change) -> Patch              // 改
meta_delete_prompt_module(id) -> Result<()>                 // 删

meta_list_profiles() -> Vec<PromptProfile>                  // 查预设
meta_create_profile(name, selections) -> ProfileId          // 增
meta_save_current_as_profile(name) -> ProfileId             // 保存当前配置为预设
meta_apply_profile(profile_id, agent_role) -> Result<()>    // 套用到某 Agent
meta_delete_profile(id) -> Result<()>                       // 删

// === ST 预设导入（核心）===
meta_import_st_preset(json_bytes) -> StPresetAnalysis
  // 返回：解析出的 prompt 列表 + 自动分类结果 + 分配建议 + 待确认项
meta_confirm_st_import(analysis_id, selections) -> Patch
  // 用户确认后，生成"新建模块 + 新建 Profile"的 patch

// === Agent 绑定（运行时切换）===
meta_get_agent_binding(role) -> AgentBinding                // 查当前绑定
meta_set_agent_profile(role, profile_id) -> Result<()>      // 切 Profile
meta_set_agent_connection(role, conn_id) -> Result<()>      // 切连接
meta_set_agent_module(role, category, module_id) -> Result<()> // 切单个模块（按钮用）

// === LLM 连接管理 ===
meta_list_connections() -> Vec<LlmConnectionSummary>        // 查（不含 key）
meta_list_connection_templates() -> Vec<ConnectionTemplate> // 查模板
meta_create_connection_from_template(tpl, api_key) -> ConnId // 从模板建（key 存 Keystore）
meta_create_connection_custom(config) -> ConnId             // 自定义建
meta_update_connection(id, change) -> Patch                 // 改
meta_delete_connection(id) -> Result<()>                    // 删
meta_test_connection(id) -> ConnectionTestResult            // 测试连通
```

#### 9.4.3 ST 预设分类规则（Meta Agent 的判断依据）

Meta Agent 用以下启发式规则分类（可被用户纠正）：

| ST prompt 特征 | 归入我们哪组 | 依据 |
|----------------|-------------|------|
| 含"人称"/"视角"/"第一/二/三人称" | perspective | 关键词 |
| 含思维链指引 + 模型名（Gemini/Claude/GLM） | cot | 模型适配 |
| 含"文风"/"叙事"/"白描"/"轻小说" 等 | style | 关键词 + 语义 |
| 含"杀"/"禁"/"反"/"抗" + 负面约束 | quality | 负面约束特征 |
| 含"字数"/"格式"/"语言"/"对白" | output | 输出规范 |
| role=system 且在 prompt_order 靠前 | Director 候选 | 位置 |
| role=system 且靠后（jailbreak 区） | Editor 候选 | 位置 |
| 含"角色扮演"/"{{char}}" 行为指引 | Subagent 候选 | 语义 |
| 无法判断 | 待人工确认 | 兜底 |

### 9.5 安全约束
- Meta Agent **不参与写作流水线**（独立）
- patch **必须用户点"采纳"**才应用
- Meta Agent 不能调用 LLM 生成工具（防 prompt injection 提权）
- **API key 永远不经过 Meta Agent**：连接的 key 由用户单独录入存 Keystore，Meta Agent 只能引用连接 ID，不能读取 key 明文

---

## 10. 前端架构（普通/高玩双视图 D19）

### 10.1 选型决策

| 选项 | 评估 | 决策 |
|------|------|------|
| Vue 3 | 生态成熟，中文社区好，学习曲线低 | ✅ **采用** |
| Svelte 5 | 编译时优化，体积小 | 备选 |
| SolidJS | 性能极致，但生态小 | ❌ |
| 原生 JS | 体积最小，但维护成本高 | ❌ |

**决策：Vue 3 + Vite + Pinia**。
理由：中文写手社区主流、生态丰富、移动端 WebView 性能足够。

UI 组件库：待定（候选 Naive UI / 自写轻量组件）。倾向**自写轻量组件**，因为要"简洁清新"，组件库容易"臃肿"。

### 10.2 双视图实现

```vue
<!-- App.vue -->
<template>
  <NormalView v-if="!powerUserMode" />
  <PowerUserView v-else />
</template>

<script setup>
const powerUserMode = useSettingsStore(s => s.powerUserMode);
</script>
```

**普通视图**：
- 大字号、卡片式、引导式
- 只显示：写作输入框、成文展示、角色选择、基础设置
- 隐藏：Agent 配置、世界书路由、向量库参数、正则、插件管理、Meta Agent

**高玩视图**：
- 紧凑布局、多面板
- 暴露全部：每个 Agent 的系统提示词、模型选择、工具集、世界书条目路由、正则脚本编辑器、向量库配置、插件 manifest 编辑、Meta Agent 对话框

### 10.3 写作流水线可视化（移动端难点）

小屏展示多 Agent 状态，用**纵向时间线 + 折叠**：

```
┌─────────────────────────┐
│ ▼ 写作中                │
│                         │
│ ✅ 导演：已规划 3 个角色 │
│    ▼ 展开 Plan          │
│                         │
│ ⏳ 子 Agent（并行）     │
│    ✅ 角色A：完成       │
│    ⏳ 角色B：生成中 67% │ ← 流式进度
│    ⏸️ 角色C：等待       │
│                         │
│ ⏸️ 编剧：等待中         │
└─────────────────────────┘
```

### 10.4 Agent 配置卡片（按钮切换 D27）

每个 Agent（导演/编剧/子模板/Meta）有一张**配置卡片**，移动端友好，按钮组一键切换。

#### 10.4.1 普通视图（精简，只露核心）

```
┌─────────────────────────────────────┐
│  🎬 导演 Agent                       │
│                                     │
│  视角   [第一人称][第二人称][第三]   │  ← 单选按钮组
│  文风   [白描 ▼]                    │  ← 下拉（太多用下拉）
│  连接   [DeepSeek ▼]                │  ← 下拉
│                                     │
│  [更多设置]   [保存为预设]          │
└─────────────────────────────────────┘
```
普通视图只露 3 个最常用按钮组（视角/文风/连接），其余收进"更多设置"。

#### 10.4.2 高玩视图（完整，全暴露）

```
┌─────────────────────────────────────────────────┐
│  🎬 导演 Agent          [当前: 小说预设v2 ▼]     │  ← Profile 切换
│  ─────────────────────────────────────────────  │
│  视角   [第一][第二][第三][群像][双视角]         │  ← 单选
│  CoT    [Gemini][Claude][GLM][DS][自由]         │  ← 单选（按模型）
│  文风   [白描][轻小说][网文][古风]...[+更多]     │  ← 单选
│  基调   [治愈][伤感][积极][消极][自定义]         │  ← 单选
│  约束   ☑杀八股 ☑抗抢话 ☐抗绝望 ☐防重复 ...    │  ← 多选（开关）
│  输出   ☑字数2000 ☐双语对白 ☑内心独白           │  ← 多选
│  连接   [DeepSeek官方 ▼]  [测试] [新建]          │
│  模型   [deepseek-chat ▼]                       │
│  ─────────────────────────────────────────────  │
│  [编辑系统提示词]  [导出预设]  [保存为预设]      │
│  [让 Meta Agent 帮我配置]                        │  ← 唤起 Meta Agent
└─────────────────────────────────────────────────┘
```

#### 10.4.3 切换的即时性

- 点按钮 → **前端立即调** `meta_set_agent_module(role, category, id)`
- 后端更新 `AgentBinding` → **下次该 Agent 调用即生效**（无需重启 App）
- 若当前正在写作中切换，**不影响本次**，下次 Agent 唤醒时生效（避免半截切换导致不一致）
- 切换后卡片右上角显示"已修改"标记，提示用户可"保存为预设"

#### 10.4.4 LLM 连接配置 UI（单独弹层）

点"连接 [DeepSeek ▼]"或"新建"，弹出连接配置：

```
┌─────────────────────────────────────┐
│  LLM 连接配置                       │
│  ─────────────────────────────────  │
│  从模板创建：                       │
│  [DeepSeek][Gemini][OpenAI]         │
│  [SiliconFlow][自定义]              │  ← 选模板自动填 URL/协议
│  ─────────────────────────────────  │
│  名称   [我的 DeepSeek           ]  │
│  URL    [https://api.deepseek.com]  │  ← 模板预填，可改
│  协议   [OpenAI 兼容 ▼]             │
│  模型   [deepseek-chat ▼]           │
│  API Key [••••••••••••••] [👁显示]  │  ← 存 Keystore
│  ─────────────────────────────────  │
│  采样   temp[1.0] top_p[0.95]       │
│  ─────────────────────────────────  │
│  [测试连接]      [保存]             │
└─────────────────────────────────────┘
```
**Key 安全**：输入框默认遮蔽；保存时直接写入 Android Keystore，UI 和配置文件只存 `SecretRef`，**明文 key 永不出现在 settings.json 或日志**。

---

## 11. 安卓部署与构建

### 11.1 工具链
- Rust + `android-ndk`（target: `aarch64-linux-android`, `armv7-linux-androideabi`, `x86_64-linux-android`）
- Tauri v2 Android（`cargo tauri android init / build`）
- JDK 17 + Android SDK（minSdk 26 / targetSdk 35，对齐你的 Companion App）

### 11.2 包体优化
- `panic = "abort"` + LTO + strip
- 向量库用纯 Rust（无 C 依赖，省 NDK 编译麻烦）
- 嵌入模型走远程（不在端上跑，省几十 MB）

### 11.3 数据存储位置
```
/data/data/com.storyforge.app/files/
├── characters/
├── world_info/
├── presets/
├── memory/
│   ├── recent.jsonl
│   ├── summaries.jsonl
│   └── vectors.msgpack
├── plugins/
└── settings.json
```

---

## 12. 日志查看系统（D29）

移动端没有 F12，排障全靠日志系统。三类日志，统一收集、面板查看、bundle 导出。

### 12.1 三类日志

| 类型 | 来源 | 内容 | 排障场景 |
|------|------|------|----------|
| **① 后端日志** | Rust `tracing` | Agent 生命周期/工具调用/流水线状态机/HTTP 错误/归档触发/panic 栈 | 后端崩溃、Agent 流程异常 |
| **② LLM 调用日志** | `infra-llm` 拦截层 | 每次调用的：连接名/模型/Profile/完整 payload/响应/token 数/延迟/错误 | 提示词效果、成本排查、API 报错 |
| **③ 前端+插件日志** | WebView console + iframe 沙箱 | JS 报错/警告、插件 `console.log`、角色卡渲染异常 | 插件开发、角色卡渲染排障 |

### 12.2 架构

```
┌──────────────────────────────────────────────────────┐
│  后端 app-logging（收集中枢）                        │
│                                                      │
│  ┌────────────┐  ┌────────────┐  ┌────────────────┐ │
│  │ tracing    │  │ LLM 拦截层 │  │ 前端日志接收   │ │
│  │ subscriber │  │ hook       │  │ command        │ │
│  └─────┬──────┘  └─────┬──────┘  └────────┬───────┘ │
│        └────────────────┼──────────────────┘         │
│                         ▼                            │
│              ┌───────────────────────┐               │
│              │  环形缓冲 + 持久化     │               │
│              │  （按类型分池，限容量）│               │
│              └──────────┬────────────┘               │
└─────────────────────────┼────────────────────────────┘
                          │ Tauri command
                          ▼
┌──────────────────────────────────────────────────────┐
│  前端日志面板（高玩模式）                             │
│  · 三类 tab 切换                                      │
│  · 级别筛选（DEBUG/INFO/WARN/ERROR）                  │
│  · 关键词搜索 + 时间范围                              │
│  · LLM 日志可展开看 payload/响应                      │
│  · [一键导出 bundle]                                  │
└──────────────────────────────────────────────────────┘
```

### 12.3 数据模型

```rust
enum LogKind { Backend, LlmCall, FrontendPlugin }

struct LogEntry {
    id: LogId,
    kind: LogKind,
    level: LogLevel,            // Debug/Info/Warn/Error
    timestamp: DateTime,
    message: String,
    // 结构化字段（便于筛选，借鉴 tracing 的 field 思想）
    fields: HashMap<String, Value>,
    // LLM 调用专属
    llm_detail: Option<LlmCallDetail>,
}

struct LlmCallDetail {
    connection_name: String,    // "DeepSeek 官方"（不含 key）
    model: String,
    profile_id: Option<ProfileId>,
    agent_role: Option<AgentRole>,
    request_payload: String,    // 完整 prompt（含正文）
    response_text: String,      // 完整回复
    prompt_tokens: usize,
    completion_tokens: usize,
    latency_ms: u64,
    error: Option<String>,
}

// 脱敏后的版本（导出 bundle 用）
struct LlmCallDetailRedacted {
    connection_name: String,
    model: String,
    profile_id: Option<ProfileId>,
    agent_role: Option<AgentRole>,
    request_payload_redacted: String,  // 正文替换为 <content N chars>
    response_text_redacted: String,
    prompt_tokens: usize,
    completion_tokens: usize,
    latency_ms: u64,
    error: Option<String>,
}
```

### 12.4 容量与持久化

- **内存环形缓冲**：每类保留最近 2000 条（移动端省内存），LRU 淘汰
- **持久化**：ERROR 级 + LLM 调用日志落盘到 `logs/`（按天滚动，保留 7 天）
- **用户可清空**：面板里有"清空日志"按钮

### 12.5 导出 bundle（关键排障功能）

用户点"一键导出"，弹出**脱敏询问**：

```
┌─────────────────────────────────────┐
│  导出日志 bundle                     │
│  ─────────────────────────────────  │
│  bundle 含：                        │
│   · 后端日志（最近 2000 条）         │
│   · LLM 调用日志（最近 200 条）      │
│   · 前端/插件日志（最近 2000 条）    │
│   · 系统信息（机型/版本/设置摘要）   │
│  ─────────────────────────────────  │
│  隐私选项：                         │
│   ☑ 隐藏写作正文（替换为占位符）     │  ← 默认勾选
│   ☑ 隐藏 API key（已永远不存）       │  ← 强制勾选
│   ☐ 隐藏角色卡/世界书内容            │
│   ☐ 隐藏连接名                       │
│  ─────────────────────────────────  │
│  [取消]              [导出 ZIP]      │
└─────────────────────────────────────┘
```

**安全默认**：写作正文默认脱敏（避免分享求助时泄露创作），API key 永远不进日志（从源头就不存）。用户自用排障可取消勾选看全文。

### 12.6 实时日志流（开发模式可选）

高玩模式可开"实时日志流"——后端用 Tauri event 持续推新日志到面板，类似 `tail -f`。默认关闭（省电），开发时开。

### 12.7 命令清单

```rust
// 查询（面板用）
log_query(filter: LogFilter) -> Vec<LogEntry>     // 按类型/级别/时间/关键词
log_get_llm_call(id) -> LlmCallDetail             // 展开看单次 LLM 调用详情
log_clear(kind: Option<LogKind>) -> ()            // 清空

// 前端上报（WebView/插件 console 转发到后端）
log_append_frontend(level, message, fields) -> ()

// 导出
log_export_bundle(redact_options) -> FilePath     // 返回 ZIP 路径，前端分享

// 实时流（可选）
log_enable_stream(kind) -> ()                     // 开
log_disable_stream(kind) -> ()                    // 关
// 后端通过 app.emit("log://entry", entry) 推送
```

---

## 13. 里程碑规划

> 每个 milestone 是可交付的：能装到安卓手机上跑。

### M0：骨架与导入（1-2 周）⭐ 起点
**目标**：能在手机上导入 ST 角色卡，看到角色信息。
- [ ] Tauri v2 Android 项目脚手架（workspace + crates）
- [ ] domain crate：Character / WorldInfo / Preset 基础模型
- [ ] infra-import：ST V2/V3 PNG+JSON 解析、内嵌世界书提取
- [ ] tauri-app：基础命令（import_character / list_characters / get_character）
- [ ] 前端：极简页面，导入按钮 + 角色列表 + 角色详情
- **验收**：导入一张真实 ST 角色卡（带世界书），完整显示所有字段。

### M1：写作流水线最小闭环（2-3 周）⭐ 核心
**目标**：2主+1子 Agent 跑通一次完整写作（无记忆系统，无向量）。
- [ ] infra-llm：HTTP client 池 + 流式（SSE）
- [ ] app-agent：单 Agent 运行时（工具调用循环 + 取消）
- [ ] app-pipeline：Director → 1 Subagent → Editor 状态机
- [ ] app-conversation：对话树基础（编辑/删除/分支 swipe/版本切换/整体重 roll，§3.7）
- [ ] app-logging：后端日志（tracing 环形缓冲）+ LLM 调用日志（payload/响应/token/延迟，§12）+ 日志面板 + 导出 bundle（默认脱敏）
- [ ] Agent 工具：search_world_info（关键词匹配，非向量）, get_character
- [ ] 前端：写作输入框 + 流水线状态展示 + 成文显示 + 版本切换条 + 重 roll/编辑/删除 + 日志面板（高玩模式）
- **验收**：输入"写一场戏"，导演拆出 1 个角色，子 Agent 演完，编剧合成成文；能对成文分支/编辑/删除/整体重 roll；日志面板能看到 LLM 调用详情，能导出脱敏 bundle。

### M2：记忆系统与向量化（2 周）
**目标**：shujuku 级三层记忆上线。
- [ ] infra-vector：hnsw_rs 集成 + 持久化
- [ ] infra-llm：嵌入 API client
- [ ] app-memory：归档器 + 召回器（§7）
- [ ] Agent 工具：search_vectors / get_recent_summary
- [ ] 前端（高玩）：总结合开关 + 已总结条目列表（D5）
- **验收**：长对话后自动归档，导演/编剧能召回远记忆。

### M3：完整流水线 + 预设/正则（2 周）
**目标**：N 子 Agent 并发 + 预设套用 + 正则生效 + 部分重 roll。
- [ ] app-pipeline：N 子 Agent 并发池（上限 4）+ 独立取消
- [ ] app-pipeline：Provenance 快照保存（每个 variant 记录子产出/Plan/种子）
- [ ] app-conversation：部分重 roll（只重跑某子 Agent / 只重跑编剧，§3.7.3）
- [ ] infra-import：ST 预设导入 + spreset 正则解析
- [ ] infra-regex：regress 引擎，输入/输出正则作用域
- [ ] 前端：子 Agent 并发视图 + 预设管理 + 正则编辑器（高玩）+ 重 roll 粒度菜单
- **验收**：导入一个真实 ST 预设，套用到导演 Agent，正则在输入/输出正确生效；能对成文做"只重跑某子 Agent"的部分重 roll，省时省 token。

### M4：插件运行时 + 角色卡渲染（2-3 周）
**目标**：能装第三方插件，能渲染带前端的角色卡。
- [ ] infra-plugin-host：iframe 沙箱宿主
- [ ] API 桥：window.storyforge（character/worldInfo/memory/variables/events/ui）
- [ ] 权限模型：manifest 声明 + 后端二次校验
- [ ] 角色卡 iframe 渲染（共用沙箱）
- [ ] app-logging：前端/插件日志（WebView console + iframe JS 报错转发到后端，§12.1 ③）
- [ ] 前端：插件管理页 + UI slot 挂载点 + 日志面板插件 tab
- **验收**：导入一个带 HTML 的 ST 角色卡，其互动界面正常渲染；安装一个简单自写插件能读角色数据；插件/角色卡的 JS 报错能在日志面板看到。

### M5：Meta Agent + 双视图打磨（2 周）
**目标**：Meta Agent 上线，普通/高玩视图完善。
- [ ] app-meta：诊断工具集 + patch 提议/采纳
- [ ] 前端：Meta Agent 对话框
- [ ] 前端：普通视图（引导式）+ 高玩视图切换
- [ ] 性能优化、包体优化、错误处理打磨
- **验收**：普通用户模式下能顺畅完成一次写作；高玩模式能调试世界书冲突并采纳 patch。

**总周期估算**：约 11-14 周（3 个月），单人全职。MVP（M0-M2）约 5-7 周。

---

## 14. 首周任务清单（M0 第 1 周）

> 目标：搭好骨架，跑通"Hello Android"。

| Day | 任务 | 产出 |
|-----|------|------|
| 1 | 环境搭建：Rust + Android NDK + Tauri v2 Android | `cargo tauri android init` 成功 |
| 2 | workspace + crates 骨架（domain/infra-import/tauri-app） | `cargo build --target aarch64-linux-android` 通过 |
| 3 | domain: Character / WorldInfo / Preset 数据模型 | 单元测试通过 |
| 4 | infra-import: PNG embed 提取 + V3 JSON 解析 | 能解析一张真实卡 |
| 5 | infra-import: 内嵌 character_book → WorldInfo 提取 | 带世界书的卡能完整解析 |
| 6 | tauri-app: import/list/get 命令 + 前端极简页 | 手机上能导入并查看 |
| 7 | 用真实 ST 角色卡（从你 VPS 拿一张测试）端到端验证 | **M0 验收** |

**首周阻塞风险**：
- Android NDK + Tauri v2 环境坑多（预留 Day 1-2 处理）
- ST 角色卡 PNG embed 的 chunk 格式细节（参考 ST 源码 `src/png/`）

---

## 15. 技术风险与对策

| 风险 | 等级 | 对策 |
|------|------|------|
| **Android NDK + Tauri v2 编译坑** | 🔴 高 | Day 1-2 专门攻坚；准备备选方案（Capacitor 套壳） |
| **多 Agent 并行的 token 成本** | 🟡 中 | 模型分层（子用廉价）；子 Agent 上限 4；默认按需启动 |
| **移动端 iframe 沙箱逃逸** | 🟡 中 | 严格 CSP + sandbox 属性 + 后端权限二次校验 |
| **ST 角色卡格式边角情况** | 🟡 中 | 早建测试集（用你 VPS 的真实卡）；PNG embed 参考 ST 源码 |
| **向量库在 Android 的性能** | 🟢 低 | M0 先暴力检索；M2 上 hnsw_rs；数据量监控 |
| **shujuku 总结 prompt 在新模型上的效果** | 🟢 低 | M2 上线后 A/B 测试，可调 prompt |
| **插件 API 设计反人类** | 🟡 中 | M4 先自写 2-3 个示例插件验证 API 再固化 |

---

## 15. 待评审的开放设计点

这些是**实现时再定**的细节，不阻塞 M0 启动：

1. **前端 UI 组件库**：自写 vs Naive UI（M0 前定）
2. **向量库持久化格式**：MessagePack vs JSON（M2 定）
3. **插件 manifest schema** 完整字段（M4 定）
4. **Meta Agent 诊断规则的可配置性**（M5 定）
5. **是否支持 RAGFlow（你 VPS 上已有的）作为远程向量源** —— 这是个潜在加分项，未来可作为高玩选项

---

## 附录 A：与 TT 的代码复用边界

| TT 模块 | 复用方式 |
|---------|----------|
| Agent profile/plan schema | **读思路，重写**（我们的 Agent 角色不同） |
| Agent 委派/并发机制 | **读思路，重写**（参数不同） |
| LLM HTTP client pool | **读思路，重写**（更简） |
| Character/WorldInfo 模型 | **读字段定义，重写**（加 Agent 工具视角） |
| PNG embed 解析 | **可直接参考**（逻辑通用） |
| 正则引擎 | **直接用 regress crate**（TT 也用这个） |
| iOS / LAN / tray / macos webview | **完全不用** |

---

## 16. 角色子 Agent 与信息隔离（D30-D31）

> 本章是对 §3.3「专属上下文包」的深化。§3.3 解决的是"上下文不互相污染"，本章进一步解决"角色只知它该知道的事"。
> 触发原因：用户的角色卡多为「赛博跑团」式（一卡多角色 + 信息差），共享 `recent_window` 会让不在场的角色"知道"它不该知道的事。

### 16.1 子 Agent 定位重申

子 Agent 是**上下文隔离容器**，不是对话接口（D30）。两个目标：
1. **信息隔离** — 林医生不该知道地下室发生了什么，除非有人告诉他
2. **人设保持** — 林医生的说话风格、性格、决策倾向始终一致，不被主 Agent 的"叙事腔"污染

子 Agent 只和导演通信，玩家永远不会直接调用它。

### 16.2 角色可见信息（character_knowledge）

每个角色维护一个**可见信息列表**，记录"这个角色知道什么"。来源四元分类（D31）：

| source | 含义 | 示例 |
|--------|------|------|
| `witnessed` | 亲眼所见 | 林医生在场时发生了爆炸 |
| `told_by_other` | 被其他人告知（记来源角色 ID） | 陈警官告诉林医生"地下室有尸体" |
| `inferred` | 自己推断的 | 林医生看到血迹推断有人受伤 |
| `backstory` | 背景设定 | 林医生是外科医生（导入时建立） |

```rust
// domain/src/character.rs（新增字段）
struct CharacterKnowledgeEntry {
    id: Id,
    campaign_id: Id,              // 会话隔离
    character_id: Id,             // 谁知道
    knowledge_text: String,       // 信息摘要（用该角色第一人称视角）
    source: KnowledgeSource,      // witnessed/told_by_other/inferred/backstory
    source_character_id: Option<Id>,  // told_by_other 时记谁告诉的
    turn_number: u32,             // 第几轮知道的
    event_id: Option<Id>,         // 关联的全局事件（可为空）
}

enum KnowledgeSource { Witnessed, ToldByOther, Inferred, Backstory }
```

### 16.3 子 Agent 上下文拼装（与现有 ContextPackage 的关系）

现有 `ContextPackage`（`domain/src/agent.rs:51`）的 `recent_window` 是**无差别共享**的。本章改为**按角色可见性过滤**：

```
ContextPackage 注入子 Agent 时：
  recent_window → 替换为「该角色可见的近期事件」(从 character_knowledge 查表)
  + 新增字段 character_knowledge: Vec<CharacterKnowledgeEntry>（该角色已知信息）
```

**关键设计**：知识注入是**确定性查表**（不调 LLM），零额外成本。只有知识抽取（§18）才调 LLM。

> 详细接口位置见 `AGENT_INTERFACES.md` §3.2。

### 16.4 与 ST 的范式差异

| | ST（提示词注入） | 我们（子 Agent 隔离） |
|---|---|---|
| 角色知道什么 | 全靠 prompt 拼接，无结构化追踪 | character_knowledge 结构化追踪，可查可改 |
| 信息隔离 | 无（所有上下文混在一个 prompt） | 子 Agent 只看 ContextPackage，物理隔离 |
| 信息传递链 | 无 | told_by_other + source_character_id 可追溯 |

---

## 17. 多角色卡模型与 Campaign（D32-D38）

### 17.1 角色模型：树形（D32）

```
CharacterCard（卡本体，导入产生，全局）
  ├── 卡元信息（name/封面/原 JSON/creator）
  └── 角色定义 CharacterDefinition（导入时识别 Agent 识别，是"模板"不是"实例"）
        每个定义: persona_prompt / behavior_rules / base_backstory
        + 附加参数（如 group: "主角团" / "反派" / "路人"）

Campaign（游玩档，用户新建，相互隔离）
  ├── 角色实例 CharacterInstance
  │     ├── 常驻实例: 引用某 CharacterDefinition + 本档的状态/知识
  │     └── 临场实例: 本档独有龙套，不属于任何卡
  ├── character_knowledge（本档知识）
  ├── Conversation 树（本档对话）
  └── story_events（本档事件线）
```

**本质**：角色分「定义」（类）和「实例」（对象）。卡里存定义，会话里跑实例，实例带本档专属知识。换会话 = 换一套实例和知识，卡定义不动。

```rust
// domain/src/character.rs（新增）
struct CharacterCard {
    id: Id,
    name: String,
    // 卡元信息
    source_json: serde_json::Value,
    // 卡内角色定义（识别 Agent 产出）
    character_definitions: Vec<CharacterDefinition>,
}

struct CharacterDefinition {
    id: Id,
    card_id: Id,                  // 属于哪张卡
    name: String,
    persona_prompt: String,       // 性格、说话风格、口头禅
    behavior_rules: String,       // 决策倾向、禁忌
    base_backstory: Vec<String>,  // 游戏开始前就知道的事（3-5 条）
    group: Option<String>,        // 附加参数："主角团"/"反派"/None
    role_type: RoleType,          // Protagonist/Supporting/Extra（常驻/配角/临场基准）
}

struct Campaign {
    id: Id,
    card_id: Id,                  // 基于哪张卡
    name: String,
    fork_from: Option<(Id, NodeId)>,  // 从哪个档的哪个节点分叉（None=空白新建）
    created_at: DateTime,
}

struct CharacterInstance {
    id: Id,
    campaign_id: Id,
    definition_id: Option<Id>,    // None = 临场角色（本档独有）
    name: String,
    // 实例可覆盖定义（D38：本档调教的人设）
    persona_override: Option<String>,
    behavior_override: Option<String>,
    instance_state: serde_json::Value,  // 本档状态（位置/状态/物品等）
    is_temporary: bool,           // 临场角色标记
}
```

### 17.2 多角色识别 Agent（D33）

导入角色卡时，跑一个独立 Agent 识别卡内角色：

```
输入：卡的 description + first_mes + character_book 全部条目
任务：语义级识别"这张卡里有哪些角色"
输出：Vec<CharacterDefinition>（每个角色的 persona/behavior/backstory）
```

> 接口位置：`AGENT_INTERFACES.md` §6.2

**为什么不靠正则/字段硬解析**：ST 角色卡的 NPC 信息藏在世界书条目里、藏在 first_mes 的脚本里、藏在 alternate_greetings 里，没有统一字段。语义级识别更鲁棒。

### 17.3 运行时新角色（D34-D35）

用户游玩中可引入卡里没有的新角色（"这时一个叫老王的路人走过来"）：

1. 导演在 Plan 时识别"这个名字在现有角色列表里不存在" → 判定是新角色
2. 导演自主决定建不建 agent + 补全 persona（不打断用户）
3. 导演建角色时标 `role_type`：常驻（Protagonist/Supporting）vs 临场（Extra）
4. 临场角色知识只存会话临时区（不进向量库），用户或导演判断"这角色要留下来"时才升级持久化

### 17.4 Campaign 隔离（D36-D37）

一个 campaign = 一个游玩档 = 一个数据隔离域。所有会话级数据按 `campaign_id` 隔离：
- character_knowledge 打 `campaign_id` 标签
- 临场角色挂 `campaign_id`
- 对话树归属 campaign
- 事件线归属 campaign

**会话分叉复用现有对话树**（D37）：`Campaign.fork_from = (源档 id, 分叉节点 id)`。分叉档共享前缀节点、独立增长新分支。已有 `MessageNode.parent_id` + variant 机制天然支持，只需加 campaign 归属标记。

### 17.5 实例改动回写（D38）

- **默认不回写**：每档独立，A 档调教的角色 B 档拿不到
- **手动固化**：用户觉得某档把角色调好了，一键"把人设变更存回卡定义"，之后新档都继承
- 固化操作：把 `CharacterInstance.persona_override` 合并回 `CharacterDefinition.persona_prompt`

---

## 18. 角色知识系统与后处理流水线（D39-D41）

### 18.1 知识存储（D39）

进现有 `infra-vector` 向量库，打标签：

```rust
// infra-vector 现有 VectorRecord 加 metadata
struct VectorRecord {
    id: Id,
    content: String,
    vector: Vec<f32>,
    keywords: Vec<String>,
    kind: VectorKind,  // ArchivedSummary / WorldInfoGreen / CharacterKnowledge（新增）
    metadata: HashMap<String, Value>,  // 新增：owner_character_id / campaign_id / source
}
```

注入子 Agent 上下文时，按 `owner_character_id` + `campaign_id` 过滤检索。

### 18.2 后处理流水线（编剧后并行，D40-D41）

```
编剧输出成文
    │
    ├─→ 剧情总结 Agent（独立，并行）→ ArchivedSummary（进记忆系统）
    │     接口位置：AGENT_INTERFACES.md §6.3
    │
    └─→ 后处理 Agent（变量解析+知识抽取合一，并行）
          ├─→ character_knowledge 写入（读成文 → 抽各角色获知信息）
          └─→ MVU stat_data 更新（仅 MVU 卡，见 §19）
          接口位置：AGENT_INTERFACES.md §6.4
```

**关键约束**：
- 总结与后处理**并行**跑，不串行阻塞
- 后处理 Agent 一次调用双产出（知识 + 变量），省调用
- 抽取范围由导演 Plan 的「在场角色列表」约束，降误分配风险

### 18.3 后处理 Agent 的输入/输出

**输入**：
- 本轮成文
- 在场角色列表（来自导演 Plan）
- MVU schema（如卡有 MVU，见 §19）

**输出**（一次调用）：
```rust
struct PostProcessResult {
    knowledge_updates: Vec<CharacterKnowledgeUpdate>,
    mvu_updates: Option<MvuVariableUpdate>,  // 非 MVU 卡为 None
}

struct CharacterKnowledgeUpdate {
    character_id: Id,
    knowledge_text: String,
    source: KnowledgeSource,
    source_character_id: Option<Id>,
}
```

**输出解析**：从成文里抽 `<knowledge>` / `<mvu_set>` 标签块（参考 MVU parseMessages 思路）。

> 详细接口位置见 `AGENT_INTERFACES.md` §6.4

---

## 19. MVU 变量框架原生兼容（D42-D43）

> 本章在对话中经过多轮推敲，核心认知有过两次重要修正：
> ① "翻译 = JS → Rust 数据结构" 是错的，**翻译 = JS 逻辑 → tool-call 描述**
> ② "渲染和逻辑混在一起" 是错的，**渲染是数据绑定（声明式），逻辑是 tool-call（翻译式），彻底分离**
> ③ "全卡二选一（要么翻译要么 WebView）" 是错的，**元素级混合（能翻译的翻译，不能的保留 JS 执行）**

### 19.1 核心原则：渲染 / 逻辑分离 + 一切逻辑归 tool-call

MVU 卡的内容分两类，处理方式完全不同：

| | 渲染（数据 → 可视化） | 逻辑（条件 → 动作） |
|---|---|---|
| 本质 | 声明式数据绑定 | 条件触发动作 |
| 谁干 | 前端原生（拿变量值画） | Agent tool-call / 用户交互 → tool-call |
| 要"翻译"吗 | 只要绑定关系（静态声明） | 要翻译成 ToolCallSpec |
| 兜底 | 复杂动画 → WebView | 翻译不了的 JS → 运行时执行 |

**关键认知**：所有"逻辑"最终都是 tool-call。翻译 = 把卡里的 JS 逻辑转成 tool-call 的描述（ToolCallSpec）。这贯彻了 P1 原则"Agent 原生，非注入原生"。

### 19.2 翻译的产物（不是 Rust 结构，是 tool-call 描述）

```rust
struct MvuTranslation {
    // 变量 schema（stat_data 定义，来自卡 initvar）
    variable_schema: Vec<VariableField>,
    
    // 数据绑定（UI 元素 ↔ 变量，纯声明式）
    ui_bindings: Vec<UiBinding>,
    
    // 规则文本（给后处理 Agent 读的自然语言规则，让它知道何时调 tool）
    update_rules: Vec<String>,
    
    // 交互动作映射（JS 事件 → ToolCallSpec）
    interactions: Vec<InteractionMapping>,
    
    // 翻译不了的 JS 片段（运行时执行兜底）
    fallback_fragments: Vec<FallbackFragment>,
}

struct UiBinding {
    element: String,           // "hp_bar"
    variable_key: String,      // "hp"
    display: BindingDisplay,   // Bar{max} / Text / Tag / Icon{mapping}
}

enum BindingDisplay {
    Bar { max: f64 },           // 进度条（血条）
    Text,                       // 纯文本
    Tag,                        // 标签（状态 buff）
    Icon { mapping: HashMap<String, String> },  // 图标映射
}

struct InteractionMapping {
    element_label: String,      // "攻击按钮"
    actions: Vec<ToolCallSpec>, // 点击后调哪些 tool
}

struct ToolCallSpec {
    tool_name: String,          // "modify_variable" / "trigger_next_turn"
    args: serde_json::Value,
}

enum InteractionAction {
    ModifyVariable { key, value_expr },
    TriggerNextTurn { hint: String },
    Multi(Vec<InteractionAction>),
    RunOriginalJs { js_snippet, description },  // 翻译不了，运行时执行
}

struct FallbackFragment {
    description: String,        // "战斗伤害计算"
    js_snippet: String,
    reason: String,             // 为什么翻译不了
}
```

### 19.3 运行时三路执行

```
导入时 Meta Agent 一次性分析卡的 JS，产出 MvuTranslation（见 §19.4）
  │
  ├─ 渲染：前端拿 ui_bindings + 变量值，原生画（零 JS）
  │
  ├─ 后处理逻辑：后处理 Agent 读 update_rules，调 tool-call（update_variable 等）
  │   · 走 run_tool_loop（和导演一致），按规则更新变量
  │   · 只在卡走"翻译路径"时注册变量 tool；走"兜底路径"的不注册（变量由 JS 管）
  │
  └─ 用户交互：前端按 InteractionMapping，点击 → 调对应 tool
      · 能翻译的：原生调 modify_variable / trigger_next_turn
      · 不能翻译的（RunOriginalJs）：嵌入式 runtime / WebView 执行原 JS 片段
```

### 19.4 Meta Agent 导入时的五合一分析

导入 MVU 卡时，Meta Agent 一次调用产出五个结果：

| 产物 | 对应字段 | 作用 |
|------|---------|------|
| 变量 schema | `variable_schema` | 初始化 CharacterDefinition/Campaign 的变量表 |
| UI 绑定 | `ui_bindings` | 前端原生渲染状态栏 |
| 规则文本 | `update_rules` | 注入后处理 Agent，让它按规则调 tool |
| 交互映射 | `interactions` | 前端按钮点击后调对应 tool |
| 兜底片段 | `fallback_fragments` | 翻译不了的 JS，运行时执行 |

**判定路径（元素级，非整卡）**：Meta Agent 对每个元素独立判断"能翻译/不能翻译"，不强制全卡统一。简单卡全部翻译（零 JS），复杂卡部分翻译部分兜底。

**置信度 + 用户可改**：翻译产物带置信度，低置信度的标注"建议验证"。用户可在高玩模式手动改 update_rules / interactions，覆盖自动翻译结果。

### 19.5 三类 MVU 卡的兼容矩阵

| 卡类型 | 渲染 | 规则 | 交互 | 路径 |
|--------|------|------|------|------|
| 纯数据驱动（多数） | ✅ 原生绑定 | ✅ 翻译成 tool-call | ✅ 翻译成 tool-call | 全原生 |
| 规则驱动（JS if/_.set） | ✅ 原生绑定 | ✅ 翻译成自然语言规则 → Agent 调 tool | ✅/⚠️ 翻译 | 全原生 |
| 重 DOM（缄默之秋类） | ⚠️ 复杂动画需 WebView | ✅/⚠️ 部分翻译 | ⚠️ 自由逻辑保留 JS | 混合（翻译 + WebView 兜底） |

实测数据（缄默之秋1.4 MVU 卡）：`document.`×179 / `getElementById`×108 / `innerHTML`×60 / 4 个 `<script>` 块共 18 万字符。这类卡的部分元素能翻译（变量绑定），部分需 WebView（复杂渲染/自由逻辑）。

### 19.6 兜底执行：嵌入式 runtime vs WebView

| 方案 | 适用 | 性能 |
|------|------|------|
| QuickJS（嵌入式 JS runtime） | 零星 JS 片段（单个函数） | 好（同进程） |
| 共享 WebView（全局一个常驻） | 大量 JS + DOM 依赖 | 中（O(1) 内存，JS 加载一次） |

**为什么共享 WebView 比每消息 iframe 好**：iframe 是 O(N) 内存（每条消息一个渲染上下文），共享 WebView 是 O(1)（全局一个计算单元）。

**依赖**：兜底执行依赖插件运行时（§8）基础设施。落地顺序：先做翻译 + 原生路径（不等插件运行时），WebView 兜底后做。

### 19.7 与 §8 插件运行时的关系

MVU 兼容与插件运行时**共用 WebView 沙箱基础设施**，但定位不同：
- 插件运行时：第三方 `.sfplugin`，自有 API，权限声明式
- MVU 兼容：复用沙箱跑 MVU 卡翻译不了的 JS 片段

> 详细接口位置见 `AGENT_INTERFACES.md` §8

---

## 20. Agent 接口索引（D44）

**详见独立文档 [`AGENT_INTERFACES.md`](./AGENT_INTERFACES.md)**。

该文档集中记录：
- 每个 Agent（导演/编剧/子Agent/角色识别/剧情总结/后处理）的 system prompt 位置
- 上下文拼装函数位置
- 输出解析函数位置
- 修改方法 + Checklist
- 常见修改场景速查

**设计原则**：所有 prompt 都是纯字符串常量/模板函数，没有隐藏拼接。改一处，效果立即可见。改 prompt 时只看 AGENT_INTERFACES.md，不用全仓库翻。

---

## 21. 叙事计划系统（任务追踪 / 长程一致性，D45）

> 解决"导演忘记三个月后的伏笔"问题。LLM 在长对话里注意力衰减，埋的伏笔会被遗忘或时间线错乱。
> 本系统是一个叙事 quest tracker：录入任务 → 每轮比对 → 条件触发注入 → 完成标记。

### 21.1 三种触发条件（都要）

```rust
enum TaskTrigger {
    Event(String),          // 事件驱动（主）："角色X得知真相"
    TurnReminder(u32),      // 轮次兜底：第 N 轮提醒
    StoryTime(String),      // 故事时钟："到第2年6月触发"（参考 MVU 日期卡片）
    Manual,                 // 只手动激活
}
// 一个任务可组合多个触发（OR 关系）
```

故事时钟（`Campaign.story_clock`）由后处理 Agent 每轮更新，随变量注入。触发匹配：
- `Event` → 后处理 Agent 判断（LLM 语义匹配）
- `TurnReminder` → 确定性比对（当前轮次 ≥ N）
- `StoryTime` → 确定性比对（当前 story_clock ≥ 目标时间）

### 21.2 任务来源（两条录入路径）

| 来源 | 谁录入 | 触发场景 |
|------|--------|---------|
| 用户显式规划 | 用户前端建 | "我希望三个月后老王复仇"——source=UserPlanned |
| 叙事中自然产生 | 后处理 Agent 抽取 | 读成文发现伏笔，source=ExtractedFromNarrative |

只让后处理 Agent 抽取会漏掉用户意图（意图不在成文里），所以两条路径都要。

### 21.3 每轮闭环（并入后处理 Agent，零额外调用）

后处理 Agent 每轮除抽知识/更新变量，还要：
1. **比对触发**：哪些 Pending/Active 任务的 trigger 满足？
2. **完成检测**：哪些任务可能完成？（输出置信度，不直接标 Completed）
3. **抽取新伏笔**：成文里有没有新任务？

```rust
struct PostProcessResult {
    knowledge_updates: Vec<CharacterKnowledgeUpdate>,
    mvu_updates: Option<MvuVariableUpdate>,
    task_updates: Vec<TaskUpdate>,  // 新增
}
```

### 21.4 完成检测的可靠性（软状态 + 用户确认）

LLM 判断"任务是否完成"会误判。两种都致命（误判完成 → 伏笔消失；误判未完 → 干扰）。解法：

- 状态非二元：`Pending / Active / LikelyCompleted(f32) / Completed / Abandoned`
- 后处理 Agent 输出置信度，不直接标 Completed
- 高置信度（>0.8）→ 提示用户确认；低置信度 → 默默继续注入

### 21.5 任务注入（确定性查表，零成本）

在导演 user message 末尾（cache 友好位置）追加触发的任务。规则在 `build_director_user_msg()` 实现：
- 遍历 Pending/Active 任务，trigger 满足 → 注入
- Completed/Abandoned → 跳过
- LikelyCompleted(>0.8) → 提示用户，不自动注入

> 详细接口位置见 `AGENT_INTERFACES.md` §9

---

## 22. cache 友好消息布局（D46）

> 利用 LLM KV cache：稳定前缀命中 cache 省钱省延迟，易变内容压尾。
> MessageLayout 是类型层护栏：编译期强制三段分离，禁止易变内容污染前缀。

### 22.1 为什么逐轮 append 本身是 cache 友好的

prefix cache 按 token 前缀匹配。每轮新消息只影响最后一条，前面的历史前缀 byte-for-byte 不变 → cache 命中。问题不在 append，而在于"把每轮都变的变量混进了前缀"。

### 22.2 三段布局

```
[1] system（稳定，整个会话不变）
    role_directive + 模块 + 蓝灯世界设定 + 工具说明
    ← cache 全命中
    
[2] 历史消息（稳定前缀，逐轮 append，写入后永不改）
    ← cache 全命中
    
[3] 当轮 user message（易变，每轮新建，用完即弃）
    写作意图 + 变量 + 故事时钟 + 任务提醒 + 在场角色状态
    ← 只影响这条
```

**每轮 cache 失效范围 = 只有最后一条消息（~1-2k token）**。稳态损失，不累积。

### 22.3 MessageLayout 抽象（编译期护栏）

```rust
pub struct MessageLayout {
    stable_system: String,
    stable_history: Vec<Message>,
    volatile_tail: VolatileTail,
}
// Builder 类型状态机：.system().history().tail()
// 调 .tail() 后不能再 .system()/.history()，编译期挡住"易变污染前缀"
```

**核心约束**：`.system()`/`.history()` 只接受不可变引用；`.tail()` 是唯一塞变量的地方。

**收益**：①防 silent bug（cache 破坏不报错只变慢）②自文档 ③可 CI 测试断言"system+history 跨轮 byte 一致"。

### 22.4 蓝灯世界设定移到 system

当前 `build_director_user_msg` 把蓝灯世界设定拼在当轮 user 里——破坏前缀稳定。改为移到 system message（指令在前、设定在后，利用 LLM 对 system 开头的 position bias）。

### 22.5 子 Agent 的布局（cache 命中率更高）

子 Agent persona/behavior 整个 campaign 不变，比主 Agent 更适合 cache：

```
[1] system（整个 campaign 稳定）
    persona + behavior + base_backstory + 工具说明
[2] pinned 知识（慢变）
    backstory 知识 + 重大揭示（pinned=true）
[3] 当轮 user message
    导演情境 + recent_window + 角色当前变量 + 任务提醒
```

**pinned 知识**：`CharacterKnowledgeEntry.pinned: bool`。backstory 自动 pin，重大揭示由后处理 Agent 标记。pin 总量有上限（防 system 膨胀），超了挤掉最老的。

### 22.6 system 变长的代价与控制

| 代价 | 程度 | 缓解 |
|------|------|------|
| 首次调用算 KV | 小 | 只首次，之后全 cache 命中 |
| 超 cache 上限（DeepSeek ~4k）挤出 | 中 | 控制总长度，超长世界设定走向量检索 |
| 注意力稀释 | 中 | 指令在 system 开头，设定在末尾 |

> 详细接口位置见 `AGENT_INTERFACES.md` §10

---

## 23. 变量层级体系（D47-D48）

> 角色/全局变量系统，参考 MVU 的 initvar + stat_data 机制：卡定义 schema + 默认值，实例只存值。

### 23.1 基础变量表（所有角色实例默认带，D48）

每个角色实例初始化时带一组默认字段：hp/mp/state/location/mood/relationship_to_player/inventory。

**默认表**（`default_character_variables()` 工厂）：

| key | label | 类型 | 默认 |
|-----|-------|------|------|
| hp | 生命值 | Int | 100 |
| mp | 体力/精力 | Int | 100 |
| state | 状态 | String | "正常" |
| location | 位置 | String | "" |
| mood | 情绪 | String | "平静" |
| relationship_to_player | 与玩家关系 | String | "陌生" |
| inventory | 物品 | Json | [] |

用户可改基础表（高玩模式），也可给单个角色/单张卡加自定义字段。MVU 卡导入时 initvar 覆盖/扩展此表。

### 23.2 三级变量

```
CharacterDefinition.variable_schema（卡级，定义有哪些字段 + 默认值）
  ↓ 实例化
CharacterInstance.variables（角色级，存当前值）
Campaign.variables（全局级，story_clock/weather/world_state 等）
  ↓ 聚合
Campaign.active_variables（注入用，每轮覆盖，用完即弃）
```

### 23.3 MVU 映射

MVU 卡的 stat_data 通常是扁平 JSON，导入时拆分（角色识别 Agent + MVU 协议层）：
- 顶层裸变量（day/money/weather）→ Campaign.variables
- 角色分组（`characters.<name>.hp`）→ CharacterInstance.variables
- initvar 定义 → CharacterDefinition.variable_schema
- 识别不准 → 标"待人工确认"（参考 §9.4 分类规则）

### 23.4 变量更新流程

| 触发 | 谁改 |
|------|------|
| 每轮后处理 Agent | 解析成文 `_.set` 指令，自动更新 |
| 用户手动（高玩模式） | set_*_variable 命令 |
| 共享 WebView 跑卡 JS | JS 副作用回调写值（重 DOM 卡） |

### 23.5 注入（cache 友好，放末尾）

`render_variables_for_injection()` 把当前变量渲染成文本，拼进当轮 user message 末尾（不进 system）。每轮失效范围只有这一段，前缀 cache 全命中。

> 详细接口位置见 `AGENT_INTERFACES.md` §7

---

## 下一步

1. **你评审本文档**，确认架构/里程碑/选型
2. 评审通过后，我创建项目骨架（workspace + crates + Tauri Android 脚手架）
3. 进入 M0 Day 1：环境搭建

需要我现在就开始搭 M0 骨架，还是你先评审文档？

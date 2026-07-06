# StoryForge 产品审查报告

> **日期**：2026-06-23
> **视角**：资深产品经理全栈综合审查
> **审查基准**：14 crate / 40,596 行 Rust + 8,228 行前端 / 87 Tauri 命令 / 51 项审计问题（已修 41，推迟 8）/ ROADMAP Phase 1-5 已完成、Phase 6 ~5%、Phase 7 未启动
> **审查方法**：通读核心代码（domain/agent.rs、app-pipeline/lib.rs、campaign.rs、mvu_translation.rs、plugin-bridge.js、PluginHost.vue、MvuStatusBar.vue 等）+ 分析三档真实角色卡（Seraphina / 缄默之秋1.4 MVU / 命定之诗 v4.1）+ 分析 ST 生态四大插件（JS-Slash-Runner / LittleWhiteBox / ST-Prompt-Template / cocktail）+ 提取 ST 本体 event_types 全集（99 个）

---

## 0. 执行摘要

StoryForge 的**核心引擎能力几乎全部具备**，但存在一个**战略级愿景鸿沟**：目标用户使用的是"缄默之秋/命定之诗"这类现代化 ST 卡，而当前 ST 兼容停在"格式兼容"，"运行时兼容"基本未接通。产品价值尚未被真实卡验证过。

本报告给出：①修正后的产品定位 ②三档验收基准（Bronze/Silver/Gold）③ST 运行时兼容的 7 个缺口 + 正则系统独立工作项 + ST 兼容层方案 ④重排后的 7 个 Sprint 路线（含 Android 主线）。

**关键修正**（通读代码后纠正的早期判断）：
- ❌ 早期判断"前端无插件接口" → ✅ 实际 `plugin-bridge.js` 已有完整的 `window.storyforge` API（权限/存储/UI槽/事件）
- ❌ 早期判断"无事件总线" → ✅ 实际前端已有事件分发，缺的是后端接线
- ❌ 早期判断"prompt 组装是哲学冲突" → ✅ 实际 `assemble_system_prompt` 已是结构化注入系统，只是未 emit
- ❌ 早期判断"原生渲染不可行" → 部分修正：状态栏原生可行（已有 `MvuStatusBar.vue`），复杂 DOM 应用走 iframe 兜底

---

## 1. 产品定位

### 1.1 定位重述

> **StoryForge 是一个以 Pipeline 为核心引擎的多 Agent 长篇叙事系统。**
> - **引擎层**：Director→Subagent→Editor→Postprocess 流水线是核心埋点，决定写作质量上限
> - **驾驶舱层**：Meta Agent 是用户驾驭引擎、诊断和修复状态的仪表盘
> - **扩展层**：开放的插件接口让社区优秀插件（MVU、生图、DS缓存、创意工坊）能快速适配；简单插件用 Meta 复刻，复杂插件通过标准接口接入并可查看/导出/修改
> - **素材层**：ST 角色卡是素材格式（含其完整 extensions 生态），不是产品形态

**目标用户**：使用"缄默之秋"这类现代化卡、追求复杂游戏状态和交互式叙事的深度玩家。Seraphina 这类传统卡不是目标用户基准。

### 1.2 与 ST 的关系

ST 角色卡是 StoryForge 的**素材格式**（像 Unity 读 FBX），不是**产品形态**（像 Unity 不是"类 FBX 查看器"）。

StoryForge 的差异化（ST 架构上做不到的）：
- **状态机**：`CampaignRuntimeContext` + `CharacterInstance` 内建，不靠脚本拼
- **信息隔离**：`knowledge_for_instance` + visibility 规则（已对抗测试验证）
- **写作流水线**：Director→Subagent→Editor→Postprocess 强编排
- **状态写回正文**：postprocess 在后端把正文反写为 variables/knowledge/tasks
- **可解释性**：Meta Agent + typed patch + provenance

### 1.3 产品形态

- **主线平台**：Android，包含所有功能
- **辅助平台**：桌面端（配置/调试场景）
- **数据策略**：本期单端，手动导入导出 Campaign bundle 跨端搬运；多端同步为 v2 规划（见 §4.6）

---

## 2. 核心体验闭环与验收

### 2.1 三档验收基准

| 档位 | 验证什么 | 对标卡 | 核心问题 |
|---|---|---|---|
| **Bronze** | 引擎能跑 | Seraphina | 状态闭环成立吗？ |
| **Silver** | 能跑带状态的卡 | 缄默之秋 | MVU 状态栏 + regex 替换 + 世界书触发，能跑通吗？ |
| **Gold** | 能跑带交互式 UI 的卡 | 命定之诗 | 嵌入式 HTML 应用 + 插件生态，能完整体验吗？ |

### 2.2 当前 ST 兼容进度（五档分类）

| ST 生态能力 | 进度 | 证据 |
|---|---|---|
| PNG tEXt chara 提取 | ✅ 已消费 | `infra-import/png.rs` |
| V2/V3 卡字段 | ✅ 已消费 | `character.rs` |
| character_book 内嵌世界书 | 🟡 部分 | Constant 注入 + search_world_info 工具；Selective 触发待补 |
| extensions 整体保留 | 🟡 已保留 | `extensions: serde_json::Value` round-trip，不消费 |
| regex_scripts | 🟡 已建模 | `infra-regex` + `preset.regex_scripts`，流水线接入待确认 |
| alternate_greetings | 🟡 已建模 | domain 有，前端无切换 UI |
| MVU bundle 执行 | 🟡 仅 postprocess | `WebViewMvuRuntime` |
| tavern_helper 脚本 | 🔴 未消费 | import 层零命中 |
| ST 宏 `${getvar}` | ❓ 子集 | JSR shim，覆盖率未验证 |
| ST 事件总线 | 🟡 前端有 | `plugin-bridge.js` 有 `events.on`，后端 `PipelineEvent` 未桥接 |
| iframe 沙箱 API | ✅ 已有 | `PluginHost.vue` + `plugin-bridge.js` 完整 |
| 插件 API（window.storyforge）| ✅ 已有 | 8 方法 + 权限 + UI 槽 + 事件 |

### 2.3 Silver/Gold 跑不通的缺口（7 个 + 正则系统独立工作项）

| # | 缺口 | 现状 | 工作量 |
|---|---|---|---|
| R | **正则系统（独立工作项，见 §2.3.1）** | 仅读 Preset 来源 + 二元 placement | **9-13 天** |
| 2 | ST 宏替换扩展到 ~30 个 | 仅 3 个 | 3-5 天 |
| 3 | first_mes/regex HTML 送进 PluginHost 渲染 | PluginHost 已有 | 3-4 天 |
| 4 | alternate_greeting 切换 UI | domain 有，前端无 | 1-2 天 |
| 5 | 事件总线接线 | 前端有，后端未桥接 | 2-3 天 |
| 6 | Prompt 组装事件暴露 | `assemble_system_prompt` 返回 String | 改返回 `Vec<PromptSegment>` + emit，2 天 |
| 7 | 世界书 Selective 触发 | Constant 已注入 | 补关键词扫描 + 绿灯激活，2-3 天 |
| 8 | H-012 trait 抽象 | ✅ 已完成：`infra-plugin-host` 不再依赖 tauri | Tauri/WebView adapter 已移动到 `tauri-app/src/mvu_webview_runtime.rs` |

### 2.3.1 正则系统（独立工作项 R）

ST 的正则脚本系统远比"Input/Output 两端替换"复杂。代码核查发现严重缺口：当前 `RegexScript`（`preset.rs:44`）只在**预设导入路径**调用 `extract_regex_scripts`，**只读 Preset 来源**，且 `placement: Vec<i32>` 被映射成二元枚举（丢信息）。

**三个来源（ST 合并优先级：Global → Preset → Scoped）**：

| 来源 | 存哪 | 现状 | 缺失影响 |
|---|---|---|---|
| Preset 脚本 | 预设 `extensions.regex_scripts` | ✅ 读 | 唯一接通的 |
| **Scoped 脚本（卡内）** | 角色卡 `data.extensions.regex_scripts` | ❌ 不读 | **缄默之秋 9 个正则全在此，导入后不执行** |
| Global 脚本 | `settings.json` | ❌ 不读 | 全局正则丢失 |

**7 个作用域（ST placement 数值）**：

| ST 作用域 | 值 | 现状枚举 | 缺失影响 |
|---|---|---|---|
| User Input | 0 | ✅ Input | |
| AI Response | 2 | ✅ Output | |
| Slash Commands | 1 | ❌ | 斜杠命令值不处理 |
| **World Info** | 3 | ❌ | **世界书条目注入前格式错乱** |
| Reasoning | 4 | ❌ | 推理模型内容块不处理 |

**瞬时性（Ephemerality，3 种持久化模式）**：

| 模式 | 行为 | 现状 | 缺失影响 |
|---|---|---|---|
| 默认 | 改写存储（不可逆）| ❌ | |
| Only Display | 只改显示不改存储 | ❌ | **MVU状态栏美化无法正常工作** |
| Alter Prompt | 只改 prompt 不改显示 | ❌ | |

**缄默之秋的正则实际用途**（需正确支持）：
- 开局造人（Output，替换标记 → HTML 表单）
- MVU状态栏（Output + Display-only，`<data_block>` → 美化状态栏，**存储必须保留原 `<data_block>` 否则反解析失败**）
- 思维链美化（Output，美化 `<think>` 块）
- 杀八股词（Output，过滤"让我们一起""值得注意的是"等套话）
- 文内选项（Output，渲染下一轮提示词的选项按钮）
- 思考隐藏 / 变量更新美化 / 界面占位符等

**拆分工作量**：

| 子项 | 工作量 |
|---|---|
| 来源合并（Global/Preset/Scoped + 优先级）| 2-3 天 |
| 作用域扩展（加 World Info/Slash/Reasoning）| 3-4 天 |
| 瞬时性（Display-only/Prompt-only/Both）| 2-3 天 |
| Depth 限制（只作用最近 N 条）| 1-2 天 |
| placement 字段重做（恢复 `Vec<i32>` 语义）| 1 天 |
| **总计** | **9-13 天** |

> sprest 插件（提取预设中的正则集中管理）本身是管理工具非运行时，不需兼容。但它揭示的"正则来源合并优先级"问题必须正确实现。

### 2.4 原生渲染路线（Native/Hybrid 分层）

已存在的设计（`MvuRouting` enum）：

```
MvuTranslation（Meta Agent 翻译卡的 HTML/JS）
  ├── ui_bindings（声明式绑定）──→ MvuStatusBar.vue 【原生，零 JS】
  ├── interactions（交互映射）──→ 原生按钮 + ToolCall
  ├── update_rules（自然语言规则）──→ 注入 postprocess Agent
  └── fallback_fragments（翻译不了的 JS）──→ WebViewMvuRuntime 【JS 兜底】

routing 判定：
  Native  = 全部原生，无 JS（最快、最安全）
  Hybrid  = 原生为主 + 部分 JS 兜底
```

**性能结论**：原生路径比 JS 快 10-40 倍。原生覆盖的是每轮高频的状态栏，JS 兜底覆盖的是低频一次性操作（开局表单/创意工坊）。性能关键路径走原生即有保证。

**翻译质量是性能的真正瓶颈**：Meta Agent 翻译越准，原生覆盖率越高，性能越好。

---

## 3. ST 生态兼容

### 3.1 事件全集分析（99 个 event_types）

从 ST 本体 `events.js` 提取的 99 个事件，分四类：

| 分类 | 数量 | 工作量 |
|---|---|---|
| 🟢 A 类 已存在/等价 | ~30 | event_type 映射（小） |
| 🟡 B 类 应该有 | ~20（含 8 个 prompt 组装钩子） | 在已有动作上加 emit（小） |
| 🔴 C 类 真不存在 | ~32 | shim 返回 noop（零） |
| ⚪ D 类 废弃 | ~3 | 忽略 |

C 类 32 个事件对应的功能是：TTS/SD 生图/多后端聚合/OAI 预设细节/密钥管理/可拖动面板等——StoryForge 架构上不做的功能。

### 3.2 插件接口三层架构（复用已有能力）

```
Layer 3: ST 插件直插（JS-Slash-Runner/LWB/PTT 等）
  - 跑在 WebView 沙箱，注入 ST API 兼容层
  - 实现 TavernHelper shim + event_types 映射
  - 让社区插件"原样加载"，零修改

Layer 2: 原生插件 API（复用 tool_center）
  - Rust trait，注册到已有 ToolRegistry
  - 示例：DS 缓存查看器、自定义正则包

Layer 1: Meta Agent 复刻
  - 自然语言→typed_patch，零代码
  - 示例：简单状态计算、自定义变量展示
```

### 3.3 技术债取舍（8 项推迟项，Android 视角重排）

| ID | 问题 | 阻碍 ST 兼容? | 阻碍 Android? | 时机 |
|---|---|---|---|---|
| H-012 | infra-plugin-host 依赖 tauri | ✅ 已修复 | ✅ 已修复 | 2026-07-06 已拆 adapter |
| H-002 | API key 明文存储 | ✅ 已修复 | 🟡 需 Android 实机验证 | 2026-07-06 已接系统凭据库 + SecretRef；Windows Credential Manager 冒烟测试通过 |
| H-013 | CampaignStore 集合级锁 + 同步 JSON I/O | 否 | 🟡 中 | S4（单 Mutex 已拆，剩余性能压测/后台 flush 待做） |
| H-014 | 同步 fs 阻塞 tokio | 否 | 🟡 中 | S4 |
| M-012 | lastConversationNode 未传入 | 🟡 影响 Meta | 否 | S3 后 |
| M-024 | patch 事务回滚 | 🟡 影响 Meta | 否 | S3 后 |
| M-022 | unwrap_or(Null) 12 处 | 否 | 否 | 可延后 |
| M-021 | LogStore 并发写入 | 否 | 否 | 可延后 |

### 3.4 兼容目标分档（可验证版）

| 档位 | 兼容度 | 做 | 不做 |
|---|---|---|---|
| Seraphina | 100% | 全字段 + 世界书 + 宏 | — |
| 缄默之秋 | ~85% | + regex + 内嵌 HTML + MVU + ZOD + ST 宏全集 | 不做 ST 事件全集（P0 的 6 个够） |
| 命定之诗 | 核心 10 项全通 + 5 项降级 | + ST 兼容层（TavernHelper shim + CDN 白名单） | 见 §3.5 |

---

## 4. 发布路线（7 个 Sprint）

### 4.1 路线总览

```
旧 ROADMAP                    新路线
─────────────────────────────────────────────────────────────
Phase 1-4 ✅ 已完成    →   保留（核心引擎 + 隔离 + Meta + 前端）
Phase 5 ✅(过早)       →   拆成 S1 验收 + S2 运行时兼容
Phase 6 ⬜ Android     →   S4 Android 前置债务 + S5 Android 主体
Phase 7 ⬜ 收口         →   S3 验收矩阵前置 + S6 发布收口
```

### 4.2 Sprint 明细

#### S1：Bronze 验收基线（3-5 天）
- 写 `docs/RELEASE-CHECKLIST.md`（Bronze 档）
- 用 Seraphina 卡走一遍：导入→建 Campaign→写 3 轮→状态闭环
- 记录"通了什么 / 没通什么"
- **完成定义**：checklist 记录每步通过/失败状态

#### S2：ST 运行时兼容 + 缄默之秋验证（18-25 天）
**阶段 A：手工翻译缄默之秋（5-7 天，用户跑测试）**
- 手工把缄默之秋状态栏翻译成 `MvuTranslation` JSON
- 聚焦：ui_bindings（HP/体力/位置/感染态）+ update_rules（ZOD 钳制）
- 开局表单归入 `fallback_fragments`（正确结果）
- 用户跑测试：`MvuStatusBar.vue` 能否正确渲染 + 数值更新
- 成功 → 产出"标准答案 JSON"
- **门 1**：失败则暂停，先扩 `MvuTranslation` 数据结构

**阶段 B：接通 6 缺口 + 写 Meta Agent skill（13-18 天）**
- 缺口 1-5 + 7（regex/宏/HTML/greeting/事件/世界书）
- 基于阶段 A 标准答案，给 Meta Agent 写提示词和 skill
- **完成定义**：缄默之秋端到端跑通（Silver 验收）

#### S2.5：ST 兼容层（7-10 天，新增）
- TavernHelper shim（38 方法转发到 window.storyforge）
- CDN 白名单 + 用户授权 UI
- iframe 允许 fetch + 网络代理
- **完成定义**：命定之诗创意工坊基础可用

> **延后项**：LittleWhiteBox 兼容（需建 script.js 最小桩模块 + 逆向 LWB 对 ST 本体的静态 import 依赖）。LWB 实际渲染的卡不多，优先级低，移至 S6 之后按需再做，不阻塞主线。

#### S3：Silver 验收 + 质量基线（3-5 天）
- 写 Silver checklist
- 用 3 张不同题材状态卡跑通
- Prompt 组装事件暴露（缺口 6）
- H-012 trait 抽象（缺口 8，已于 2026-07-06 完成）
- **门 2**：部分失败则补缺口，不进 Android

#### S4：Android 前置债务（5-7 天）
- H-002 API key → keyring
- H-013 CampaignStore 写入性能压测；集合级锁已完成，必要时继续做后台 flush / `spawn_blocking`
- H-014 同步 fs → spawn_blocking
- **完成定义**：4 项技术债关闭，test 全绿

#### S5：Android 主体（10-15 天）
- Android 构建链路基线（按 PLAN-ANDROID 阶段 1）
- 文件导入路径（FileProvider）
- WebView 生命周期管理（onPause/onResume）
- 流式显示性能 + 断网重试
- Bronze + Silver checklist 在 Android 跑一遍
- **门 3**：低端机卡顿则加降级策略，延后 Gold
- **完成定义**：Android 端跑通缄默之秋（主线 MVP）

#### S6：Gold 验证 + 发布收口（10-15 天）
- Gold checklist（命定之诗级）
- 命定之诗插件兼容测试
- 桌面端 + Android 发布包
- 首次使用文档
- 长会话性能/成本检查
- 手动导入导出跨端搬运（v2 同步的折中）
- **完成定义**：Gold 档卡可玩，发布包就绪

**总计**：约 56-82 天（S1-S6 累加）。**Android MVP（S5 结束）：约 39-57 天**

### 4.3 Gold 档兼容定义（可验证）

核心 10 项全通：
1. 导入卡（PNG + 世界书 + extensions）
2. 显示开场画面（first_mes HTML 基础渲染）
3. 状态栏原生渲染
4. 数值随轮次更新
5. 写作主流程
6. 状态写回 + 知识隔离
7. alternate_greeting 切换
8. 世界书关键词触发
9. 基础 ST 宏替换
10. regex_scripts 触发

5 项降级（§3.5）：
11. 开局表单 → iframe JS 兜底（可用）
12. 创意工坊 → ST 兼容层（基础可用）
13. JSR/LWB/PTT → 部分加载
14. CDN 内容 → 白名单授权
15. 主题/地图 → 随创意工坊降级

### 4.4 ST 兼容层方案（S2.5）

**三重依赖**：
1. ST 本体全局 API（script.js 导出）→ TavernHelper shim 转发到 window.storyforge
2. 远程 CDN 脚本 → 白名单 + 用户授权
3. iframe 沙箱执行 → 已有 PluginHost.vue

**TavernHelper shim**（38 方法转发，非重实现）：
```javascript
globalThis.TavernHelper = {
  getVariables: (name) => window.storyforge.variables.get(campaignId, name),
  setVariables: (name, value) => window.storyforge.variables.set(campaignId, name, value),
  eventOn: (event, cb) => window.storyforge.events.on(event, cb),
  // ...38 个方法，大部分是 1 行映射
}
```

**诚实限制**：
- 兼容性需持续维护（ST 生态在演进，shim 要版本化可更新）
- 性能上限是 ST 桌面端水平，移动端需降级
- LWB 兼容延后（script.js 桩 + 逆向 import，按需再做）

### 4.5 明确不做事项

- 全原生渲染复杂 DOM 应用（声明式表达不了命令式业务逻辑）
- 100% 复刻 ST 事件全集（32 个 C 类架构上不存在）
- 自建 CDN 内容市场（用原生插件市场替代）
- 重写 JS-Slash-Runner / LWB（实现它依赖的 shim 而非重写它——见下方设计决策）
- 知识传播引擎方向 2/4/5（待立项，不阻塞发布）
- 多端云同步（v2 目标，本期手动导入导出折中）

#### 4.5.1 设计决策：不重写 JSR，走 shim 转发

曾评估"重写 JSR 成原生运行时"是否提升性能。结论是**不重写**，理由：

1. **性能收益可忽略**：重写能优化的只是"变量推给渲染层"这段（postMessage 序列化 vs 直接内存），约 1-5ms。而一轮写作的瓶颈是 LLM 生成（3000-10000ms）和状态栏 DOM 渲染（200-400ms 移动端），这两段重写后**完全不变**（渲染引擎还是 WebView）。收益占比 0.01%-0.17%，用户感知不到。
2. **真正的性能优化是原生渲染覆盖率**：状态栏走原生 `MvuStatusBar.vue`（<10ms）vs 走 iframe（200-400ms），差距 20-40 倍。这个优化**已经做了**（MvuTranslation 的 Native 路由），比重写 JSR 有效得多。
3. **重写 = 放弃"ST 兼容"卖点**：重写后 JSR 的插件/教程/技巧不再通用，用户会说"StoryForge 的 JSR 和真 JSR 不一样"。失去兼容性这个核心定位。
4. **维护负担无止境**：JSR 持续更新（已到 4.8.11，auto_update=true），重写要手动跟进每个新 API；shim 转发只需保证核心几个方法（variables/events）稳定，JSR 自己更新自己。

**策略**：性能靠原生翻译覆盖率，不靠重写运行时。状态栏（高频）→ 原生；开局表单/创意工坊（低频）→ iframe + shim。

### 4.6 多端同步评估

**档位 A（桌面↔手机，同一用户）**：中等难度，3-4 周。数据模型有利（全 JSON），冲突主要是追加型。实现路径：自建同步服务 / Git 当传输层 / CRDT 库。建议 v2。

**档位 B（多用户实时协作）**：极难，不评估。非需求。

**本期折中**：S6 加手动导入导出（复用 `export_campaign_bundle`），文档注明 v2 规划。

---

## 5. Meta Agent Skill 草案框架

### 5.1 目标

让 Meta Agent 能自动复现"手工翻译缄默之秋"的标准答案——输入卡的 HTML/JS，输出结构正确的 `MvuTranslation`。

### 5.2 Skill 结构

```
skill: mvu-card-translation
  ├─ 输入：卡的 regex_scripts / first_mes / tavern_helper
  ├─ 分析维度：
  │   ① 变量 schema（从 ZOD / initvar 提取）
  │   ② UI 绑定（哪些变量需要 bar/text/tag/icon 展示）
  │   ③ 交互映射（按钮 → 改变量/触发下轮）
  │   ④ update_rules（变量计算/钳制逻辑的自然语言描述）
  │   ⑤ fallback_fragments（翻译不了的 JS，标注 reason）
  │   ⑥ routing 判定（Native / Hybrid）
  │   ⑦ 置信度评估
  ├─ 输出：MvuTranslation JSON
  └─ 验证基准：缄默之秋标准答案 JSON（S2 阶段 A 产出）
```

### 5.3 提示词要点

- 先识别卡的"状态栏"部分（regex 匹配 `<data_block>` 或类似标记）
- 状态栏 → 优先翻译为 ui_bindings（原生路径）
- 含 `document.createElement`/`addEventListener`/复杂计算的 → 归入 fallback_fragments
- 每条 fallback 标注：description（干什么）/ reason（为什么翻译不了）/ js_snippet（原代码）
- routing 判定：fallback 为空 → Native；非空 → Hybrid
- 置信度：翻译成功的 binding 数 / 总 binding 数

### 5.4 验证方法

用缄默之秋标准答案做 diff：
- 变量 schema 匹配率
- ui_bindings 结构匹配率
- fallback_fragments 数量合理性
- routing 判定一致性

---

## 6. 附录

### 6.1 关键代码位置索引

| 能力 | 文件 | 行 |
|---|---|---|
| 流水线状态机 | `crates/app-pipeline/src/lib.rs` | 221-547 |
| PipelineEvent 定义 | `crates/domain/src/agent.rs` | 245-307 |
| Campaign（运行时真相源）| `crates/domain/src/campaign.rs` | 15-118 |
| CharacterInstance（信息隔离）| `crates/domain/src/campaign.rs` | 122-232 |
| assemble_system_prompt | `crates/domain/src/prompt_module.rs` | 153-196 |
| MvuTranslation（原生/兜底路由）| `crates/domain/src/mvu_translation.rs` | 28-125 |
| WebViewMvuRuntime | `crates/tauri-app/src/mvu_webview_runtime.rs` | 全文 |
| 插件 API（window.storyforge）| `frontend/src/plugin-bridge.js` | 35-131 |
| PluginHost（iframe 沙箱）| `frontend/src/components/PluginHost.vue` | 全文 |
| MvuStatusBar（原生渲染）| `frontend/src/components/MvuStatusBar.vue` | 全文 |
| postprocess 编排 | `crates/app-agent/src/pipeline_postprocess.rs` | 47-100 |

### 6.2 角色卡复杂度光谱

| 维度 | Seraphina | 缄默之秋1.4 | 命定之诗v4.1 |
|---|---|---|---|
| description | 2851 字 | 360 字 | 0 字 |
| first_mes | 785 字叙事 | 28KB HTML 应用 | 4 字 + 脚本渲染 |
| alternate_greetings | 0 | 1 | 6 |
| regex_scripts | 0 | 9 个/420KB | 0 |
| tavern_helper | 0 | 2（ZOD+MVU）| 6（创意工坊63KB）|
| character_book | 0 | 212 条内嵌 | CDN |
| 状态管理 | 无 | ZOD + MVU | MVU + 地图 |

### 6.3 ST 事件全集分类（99 个）

- 🟢 A 类 已存在/等价：~30（GENERATION_STARTED/ENDED、MESSAGE_RECEIVED、CHAT_LOADED、CHARACTER_*、STREAM_TOKEN 等）
- 🟡 B 类 应该有：~20（含 8 个 prompt 组装钩子：GENERATE_BEFORE/AFTER_COMBINE_PROMPTS、CHAT_COMPLETION_PROMPT_READY 等；WORLDINFO_*、TOOL_CALLS_*、GROUP_*）
- 🔴 C 类 真不存在：~32（TTS_*、SD_PROMPT_PROCESSING、SECRET_*、OAI_PRESET_*、MAIN_API_CHANGED、MOVABLE_PANELS_RESET 等）
- ⚪ D 类 废弃：~3（SMOOTH_STREAM_TOKEN_RECEIVED、SETTINGS_LOADED_BEFORE/AFTER）

---

> **下一步**：本报告经用户审阅确认后，进入 writing-plans skill 创建 S1（Bronze 验收）的实施计划。

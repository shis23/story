# 写卡工具接入 StoryForge 可行性分析与方案构想

> 状态：分析稿（2026-07-22）  
> 范围：A 从零创作 / B 小说改编 / C 改卡补卡  
> 输入资产：
>
> - 玉藻前一键写卡器（ST tavern_helper 远程脚本）
> - `明月秋青写卡预设-标准+辅食.json` / `明月秋青-V10086.json`
> - `写卡知识库.json`
> - `明月小说文风蒸馏总结工具.zip`
>
> 硬约束：
>
> 1. 不破坏 Campaign 主线（写作/Accept/状态闭环优先）
> 2. 不把 ST 运行时当产品形态；ST 卡仍是素材格式
> 3. 不接受“嵌一个 ST 写卡 iframe 就算完成”
> 4. 产物必须能进入现有 `Character` / `CharacterCard` / `CharacterDefinition` / 世界书 / MVU 导入导出链路

---

## 1. 结论先讲

### 1.1 总判断

**可行，而且值得做。**  
但正确姿势不是“把玉藻前/秋青子/文风工具整包塞进 App”，而是：

> **拆方法论与流水线，原生实现 Card Studio；把 ST 写卡生态当参考实现与资料源，不当宿主依赖。**

ABC 三条链路可以统一成一个产品：

```text
素材输入（想法 / 小说 / 已有卡）
  → 蒸馏与结构化（文风、角色、世界观、关系、伏笔）
  → 分阶段写卡（明月秋青方法论）
  → 校验与自查（写卡知识库）
  → 组装 ST 兼容草稿
  → 导入为 Character/Card → 可选创建 Campaign
```

### 1.2 一句话产品定位

**Card Studio = StoryForge 的上游素材生产台。**

- Campaign Runtime = 用卡写故事  
- Card Studio = 造卡 / 改卡 / 从小说炼设定  
- 二者共享 LLM 连接、存储、导入导出、Meta 检查能力，但**不共用写作主链语义**

### 1.3 不做的事（一期明确砍掉）

- 原样运行玉藻前一键写卡器 CDN 脚本
- 原样运行秋青子“伪 IDE”酒馆助手脚本
- 复刻完整 ST Prompt Manager 开关矩阵作为主 UI
- 在 Android 上承诺完整复杂前端美化写卡（可后置）
- 让写卡流水线劫持 Director/Editor 正常正文生成

---

## 2. 当前项目能力盘点

### 2.1 已有、可复用

| 能力 | 现状 | 对写卡的意义 |
| --- | --- | --- |
| ST 卡导入 PNG/JSON | `import_character` 完整保真 `raw_card_json` / extensions / 世界书 | C 改卡的输入出口 |
| 卡列表与角色识别 | `list_cards` / `get_card` / `extract_characters` | 写卡完成后进入现有卡库 |
| 世界书读写 | 角色世界书 + Campaign 世界书 API 已有 | 写卡过程可直接维护条目 |
| 预设导入/激活/模块化 | `import_preset` / `set_active_preset` / `import_preset_as_modules` / Meta 分类 | 明月秋青可先当资料与模块源 |
| MVU 分析/应用 | `meta_analyze_mvu_card` / preview-apply schema | C 线补 MVU 可复用 |
| 导出 | `export_st_card_png` / campaign ST cards / bundle | 写卡结果可回流 ST 生态 |
| LLM 连接配置 | OpenAI-compatible | 写卡 Agent 直接复用 |
| Meta patch 闭环 | propose → preview → accept | 改卡建议不要直接写盘 |
| 插件桥 / Card Shell | 进行中，偏运行时渲染 | 与“写卡生产”正交，后期可联动预览 |

### 2.2 明显缺口

| 缺口 | 说明 |
| --- | --- |
| 无原生“创建空卡 / 草稿卡”一等公民 | `CardLibrary` 空态只说“请先导入角色卡” |
| 无卡字段结构化编辑器 | 有世界书局部编辑，无 description/personality/scenario/first_mes/MVU 一体化编辑 |
| 无写卡会话/阶段状态机 | 预设可导入，但不理解“当前在写世界观还是调色盘” |
| 无小说蒸馏流水线 | 文风/总结工具完全在仓库外 |
| 无写卡知识检索 | 知识库文件在根目录，未入库 |
| 无“生成结果 → ST JSON 组装器” | 现在只有 import/export，没有 generate-to-card compiler |

### 2.3 与产品意图的关系

- `docs/INTENT.md`：ST 卡是兼容输入，Campaign 才是主线真相源  
- `docs/PRODUCT-REVIEW`：ST 是素材格式，不是产品形态  
- 因此写卡模块应产出**高质量素材**，而不是把 StoryForge 变成第二个 SillyTavern 写卡器 UI 克隆

这是优点，不是障碍：  
StoryForge 可以做出 **比 ST 写卡器更结构化** 的生产台（schema、阶段、校验、一键进 Campaign）。

---

## 3. 外部写卡资产拆解

### 3.1 玉藻前一键写卡器

```json
{
  "type": "script",
  "content": "import 'https://cdn.jsdelivr.net/gh/.../一键角色卡写卡器/index.js'"
}
```

本质：

- ST 全局/角色脚本入口
- 真正 UI/逻辑在远程 ESM
- 依赖酒馆助手 API（角色卡、世界书、消息、按钮、生成请求等）

价值：

- 交互范式：一键入口、分步引导、结果写回卡
- 产品预期：用户想要“少配置、多引导”

风险：

- 远程依赖、版本漂移、离线不可用
- API 面与 StoryForge 宿主不一致
- 安全面：任意远程 JS

**结论：只吸收 UX 与流程，不运行脚本本体。**

### 3.2 明月秋青写卡预设（最高价值）

两份预设基本同构（61 prompts），差异主要在个别条目开关/小内容。

内嵌脚本：

- `MAG老师偷偷在后台帮你写卡` → Lorebook tool-call 远程模块
- `秋青子` → 伪 IDE 远程模块 + 按钮“打开明月秋青”

真正可沉淀的是 **prompt 方法论**，不是脚本：

| 阶段族 | 代表条目 | 产出 |
| --- | --- | --- |
| 基础 | 角色基础 | 姓名/外貌/背景/关系（不含性格） |
| 性格 | 调色盘 / 三面性 / 二次解释 / 多阶段 | 去标签化性格结构 |
| 世界 | 世界观 / 世界书评估 | 世界书条目草案 |
| 周边 | 衣柜 / NPC / 速览 / 开场白 | 周边设定与 first_mes |
| 系统 | MVU 结构/初始/更新/输出/状态栏 | 变量系统 |
| 高级 | EJS / 前端美化 / 手枪卡 / NSFW 调色盘 | 可选增强 |
| 运行约束 | 绝对零度、标签规范、输出格式、思考链包装 | 生成纪律 |

关键设计思想（必须保留）：

1. **协作式写卡，不替用户乱编**（用户没说的不写）
2. **分阶段开关**，一次只激活相关知识
3. **模板标签**（`<template_basic>` 等）+ 变量记录当前任务
4. **自查清单**（评估/自查条目）作为 Quality Gate

**结论：把明月秋青拆成 Card Studio 的 Stage Pack / Prompt Pack。**

### 3.3 写卡知识库.json

- 形态：ST 世界书 `entries`，37 条，约 8 万字
- 全部 `disable: true`（资料库，不是运行注入）
- 内容分层：
  - 酒馆助手 API 手册（00–19）
  - EJS 语法与多阶段人设
  - MVU / Zod / 状态栏
  - 世界书配置与自查
  - 前端美化自查

对 StoryForge：

- **不要**当角色世界书蓝灯注入
- **要**当 Card Studio / Meta 的只读知识索引（RAG 或按阶段挂载）

可直接映射：

| 知识条目 | 服务阶段 |
| --- | --- |
| 世界书配置指南 / 世界观自查 / 世界书评估 | 世界观与条目生成 |
| MVU_ZOD / MVU自查 / 初始变量相关 | MVU 阶段 |
| EJS* | 多阶段人设 / 动态条目 |
| 前端美化自查 | 前端壳阶段 |
| 一般条目自查 | 任意条目 Quality Gate |
| API 手册 | 仅在“兼容 ST 脚本/导出检查”时使用，不进普通用户写卡主路径 |

### 3.4 明月小说文风蒸馏总结工具

独立 Node 本地工具：

- 文风蒸馏：片段 → 阶段公式书 → 总公式书 → 可执行文风 prompt
- 剧情总结：小总结 → 大总结 → 超级总结，并维护
  - 主线、伏笔、角色、世界观、关系网、体系、时间线、地点势力、未解问题
- 自带 OpenAI-compatible 代理与工程状态 `project_state.json`

这几乎是 **B 线（小说改编）的现成流水线规格书**。

**结论：移植“状态机 + 产物模型 + prompt 协议”，不要嵌它的 HTML UI。**  
StoryForge 已有 LLM 层，不必再起一个 28632 端口小服务器。

---

## 4. ABC 场景统一模型

### 4.1 统一领域对象

建议新增（概念层，不一定一次全落地）：

```text
CardProject
  id, name, mode: FromScratch | FromNovel | FromExistingCard
  source_refs: [novel_doc_id? | source_character_id?]
  stage_state: { stage_id, status, notes }
  artifacts: CardArtifacts
  revision, created_at, updated_at

CardArtifacts
  brief                  # 用户意图/约束
  style_profile          # 文风公式/短 prompt
  worldview_entries[]    # 世界书草案
  characters[]           # 多角色档案草案
  relationships[]
  openings[]             # first_mes / alternate
  mvu_schema?            # zod/js 或内部 schema
  mvu_init?
  mvu_update_rules?
  frontend_assets?       # 后置
  self_check_reports[]
  compile_warnings[]

CardStagePack
  stages[]: id, title, required_inputs, prompt_modules, output_schema, checkers
```

编译目标：

```text
CardArtifacts
  → ST V2/V3 JSON (+ optional PNG)
  → Character (import 路径)
  → CharacterCard + CharacterDefinitions（可触发 extract 或写卡时直接生成定义）
```

### 4.2 三条入口，一套后端

| 模式 | 输入 | 先跑什么 | 再跑什么 |
| --- | --- | --- | --- |
| A 从零 | 用户 brief / 问卷 | 阶段向导 | 逐段生成 artifacts |
| B 小说 | txt/md 小说 | Distill pipeline | 把蒸馏产物预填 artifacts，再人工精修阶段 |
| C 已有卡 | character_id / 导入文件 | 反解析 raw_card → artifacts | 针对缺口阶段补齐 |

C 的反解析是关键能力：  
`raw_card_json + embedded_world_info + extensions` → 可编辑 `CardArtifacts`。  
没有它，C 只能做“旁边重新生成再手工粘贴”。

---

## 5. 方案对比

### 方案 1：ST 脚本托管（嵌写卡器）

在插件 iframe / CardShell 里加载玉藻前/秋青子远程脚本，尽量 shim API。

- 优点：表面上最快“能点开”
- 缺点：
  - 与产品定位冲突
  - API 缺口巨大（消息楼层、预设操作、角色卡写回、slash…）
  - CDN/离线/安全/版本不可控
  - Android WebView 更脆
  - 结果难结构化进 `CharacterDefinition`
- 评级：**不推荐作主路径**；最多做研究沙盒

### 方案 2：仅资料化（导入预设 + 知识库，人工用聊天写）

把明月秋青当普通预设，知识库当世界书，用户自己聊。

- 优点：实现量最小
- 缺点：
  - 没有阶段状态机
  - 没有编译成卡
  - 没有自查闭环
  - ABC 体验割裂
- 评级：**可作为第 0 周热身，不是产品方案**

### 方案 3：原生 Card Studio（推荐）

吸收明月秋青阶段法 + 文风蒸馏流水线 + 知识库自查，做成 StoryForge 原生模块。

- 优点：
  - 与 Campaign 数据模型对齐
  - 可测试、可离线打包 prompt 资产
  - A/B/C 统一
  - 能做比 ST 更好的结构化导出与一键开档
- 缺点：
  - 一期工作量明显
  - 需要仔细做编译器与校验
- 评级：**主推荐**

### 方案 4：混合（原生主路径 + 可选高级脚本实验床）

主路径走方案 3；另开“兼容实验”开关研究远程写卡脚本。

- 只在桌面调试场景有意义
- 不进入默认用户路径
- 评级：主路径稳定后可考虑，不进 MVP

**推荐：方案 3，分四期交付。**

---

## 6. 推荐架构

```text
┌──────────────────────────────────────────────┐
│ Frontend: Card Studio                         │
│  入口：从零 / 小说 / 已有卡                     │
│  阶段轨 + 产物面板 +  diff/预览 + 自查报告       │
└───────────────────────┬──────────────────────┘
                        │ Tauri commands
┌───────────────────────▼──────────────────────┐
│ app-cardstudio（建议新 crate 或 app-meta 子域） │
│  StageRunner / Distiller / Compiler / Checker  │
└───────┬───────────┬───────────┬──────────────┘
        │           │           │
   LLM (infra)  CardProject  Knowledge Index
        │        Store        (写卡知识库)
        │           │
        ▼           ▼
  ST JSON Compiler → import_character / update draft card
        │
        ▼
  CharacterCard → extract_characters → create Campaign
```

### 6.1 与现有模块边界

| 模块 | 职责 |
| --- | --- |
| `app-cardstudio`（新） | 写卡会话、阶段 prompt、蒸馏、编译、自查 |
| `infra-import` | 只负责 ST parse/serialize 保真；编译结果走它 |
| `app-meta` | 复用 health/MVU analyze；不把写卡主状态塞进 Meta 聊天 |
| `app-pipeline` | **不改主链**；写卡不走 Director 正文流水线 |
| `preset_store` | 可导入明月秋青作资产包来源；运行时不依赖 active preset |
| frontend CardLibrary | 增加“新建写卡项目 / 从卡创建写卡项目”入口 |

### 6.2 Prompt 资产落地方式

不要继续让用户“激活 ST 写卡预设来写正文”。  
改为仓库内版本化资产：

```text
assets/cardstudio/
  stage-packs/mingyue-qiuqing/v1/
    pack.json
    stages/*.md
    checkers/*.md
  knowledge/
    write-card-kb.v1.json   # 从写卡知识库规范化
  distill/
    style-dimensions.json
    summary-pipeline.json
```

从明月秋青提取时：

- 去掉破限/防429/吃思维链等与 ST 聊天宿主强绑定的条目（或降为可选 advanced）
- 保留创作原则、模板、输出格式、阶段知识
- 将 `{{addvar::template_knowledge::...}}` 之类 ST 宏改写为 StageRunner 本地上下文装配

### 6.3 编译器（成败关键）

`compile(CardArtifacts) -> StCardDraft`

最小必出字段：

- `name`, `description`, `personality`, `scenario`, `first_mes`
- `character_book.entries[]`（keys/constant/order/position/content）
- `alternate_greetings?`
- `extensions` 占位（MVU / regex 后置写入）

编译规则：

1. 角色基础 → description 结构化文本
2. 调色盘/三面性 → personality（或拆到世界书多阶段条目）
3. 世界观条目 → character_book
4. 开场白 → first_mes / alternate
5. MVU 产物 → extensions 约定路径（先对齐现有 `meta_analyze_mvu_card` 能认的形态）
6. 产出 `compile_warnings`（缺 keys、空条目、过长、冲突）

**先保证 JSON 可 import 且 round-trip 不丢关键块，再追求“高级卡完整度”。**

### 6.4 自查器

把知识库里的自查条目变成可执行检查，至少分三级：

- L1 结构：必填字段、条目 keys、禁用空 content
- L2 方法论：是否把性格写进基础、是否蓝绿灯滥用、是否替用户编造
- L3 高级：MVU schema 合法性、EJS 装饰器、前端正则只定位不传数据

L1 规则化；L2/L3 可用 LLM + 知识片段。

---

## 7. 分期路线（ABC 都覆盖）

### Phase 0 — 资产沉淀（0.5–1 周）

目标：把外部工具变成可版本管理的内部资产，不改主流程。

交付：

1. 规范化导入：
   - 写卡知识库 → `assets/cardstudio/knowledge`
   - 明月秋青阶段 prompt 抽取清单（人工校对）
   - 文风/总结 prompt 协议抽取
2. 文档化阶段地图与输出 schema
3. 明确授权/署名策略（三明月等作者标识如何展示与是否可再分发）

验收：

- 不依赖 CDN 即可阅读全部写卡阶段说明
- 有一张“阶段 → 输入 → 输出 → 检查”表

### Phase 1 — MVP：A + 最小 C（2–3 周）

目标：能从零做出**可导入的基础卡**，也能打开已有卡做字段级补强。

范围：

- `CardProject` 存储（JSON 即可）
- 阶段：Brief → 角色基础 → 性格 → 世界观（简）→ 开场白 → 编译
- 产物面板可编辑
- `compile → import_character`（或 `save_card_draft`）
- C：从 `raw_card_json` 反填基础字段 + 世界书条目，支持补“开场白/角色基础/世界观”
- CardLibrary 入口：`新建写卡` / `从该卡修订`

不做：

- 完整 MVU 生成
- 前端美化
- 小说长文蒸馏

验收金标：

1. 用户 20 分钟内从一句话人设得到可导入 JSON/PNG
2. 导入后可 `extract_characters` 并创建 Campaign
3. 已有简单卡可补一条世界书与新开场并导出

### Phase 2 — B 小说蒸馏（2–3 周）

目标：接文风工具流水线。

交付：

- 小说文档入库与切分（先 txt/md）
- 文风：片段报告 → 阶段公式 → final_formula / final_prompt
- 总结：小/大/超级 + 角色/世界观/关系/伏笔账本
- 一键“预填 CardProject artifacts”
- 长任务进度、断点续跑、artifact 落盘

验收：

- 用一本中短篇样例跑通蒸馏
- 预填后进入 Phase1 阶段精修，输出可玩卡
- 文风 prompt 可挂到后续 Campaign 写作 profile（可选接线）

### Phase 3 — 深度 C + 系统卡（2–4 周）

目标：补齐现代复杂卡生产能力。

交付：

- MVU：结构脚本 / init / update rules 生成 + 对接现有 MVU analyze/apply
- 多角色卡：直接产出多个 `CharacterDefinition` 草案
- 世界书高级：蓝绿灯策略、递归、概率、分组
- 自查报告 UI + 一键修复建议（Meta patch 风格）
- 导出前兼容矩阵检查（复用 import compat 思想）

后置（Phase 4+）：

- EJS 多阶段人设
- 前端美化/状态栏模板
- 桌面端脚本实验床（非默认）

---

## 8. 数据流示例

### A 从零

```text
用户 brief
 → Stage: 角色基础（LLM + template_basic）
 → 用户确认/改稿
 → Stage: 性格调色盘（强制用户参与衍生）
 → Stage: 世界观（A/B/C 类型分流）
 → Stage: 开场白大纲
 → compile ST JSON
 → import → CardLibrary
 → extract → 开 Campaign
```

### B 小说

```text
导入小说
 → Distill style + summary books
 → 自动生成 characters/worldview/relations/style_profile
 → 创建 CardProject(mode=FromNovel)
 → 用户在阶段轨精修
 → compile → import
```

### C 改卡

```text
选择已有 Character/Card
 → reverse_parse 到 CardArtifacts
 → 诊断缺口（无世界书 / 无 MVU / 开场弱 / 性格标签化）
 → 只跑缺口阶段
 → diff 预览
 → apply 到 draft → export 或覆盖策略（默认另存，不直接毁掉原卡）
```

**覆盖策略建议：默认“另存为新卡/新项目”，原卡只读；用户显式确认才覆盖。**

---

## 9. 风险与对策

| 风险 | 影响 | 对策 |
| --- | --- | --- |
| 范围膨胀到第二个 ST | 拖垮 Campaign 主线 | 分期；写卡独立模块；主链不改 |
| 直接跑远程脚本 | 安全/兼容/不可测 | 禁止默认路径加载 CDN 写卡器 |
| 编译器质量差 | 导出卡 ST 打不开或丢字段 | 复用 `infra-import` round-trip 测试 + fixture |
| 长小说蒸馏成本高 | 费 token/时间 | 切分、缓存、断点、可调阶段字数 |
| 授权与署名 | 法律/社区争议 | 资产来源与作者标识单独声明；可插拔 pack |
| 与 active preset 混淆 | 用户以为写卡预设影响正文 | UI 文案隔离：Card Studio 预设 ≠ 写作预设 |
| Android 性能 | 大 JSON/长任务 | 桌面先做蒸馏；移动端先 A/C 轻量 |

---

## 10. MVP 技术切片（供后续 plan）

### 后端

1. `CardProjectStore`（`data/card_projects/*.json`）
2. commands：
   - `cardstudio_list_projects`
   - `cardstudio_create_project`
   - `cardstudio_get_project`
   - `cardstudio_update_artifact`
   - `cardstudio_run_stage`
   - `cardstudio_run_checks`
   - `cardstudio_compile`
   - `cardstudio_import_compiled`
   - `cardstudio_reverse_parse_character`
3. `StageRunner`：装配 system/dev/user，调用现有 LLM client
4. `StCardCompiler` + 单元测试 fixture

### 前端

1. `CardStudioScreen`（可先桌面宽屏）
2. 左：阶段轨；中：对话/生成；右：产物与 diff
3. CardLibrary 增加入口按钮
4. 不进入 PluginHost 默认加载写卡脚本

### 测试

1. 编译最小卡 → import → export round-trip
2. reverse_parse 保真基础字段与世界书条数
3. stage prompt 装配快照测试（不含真实 LLM）
4. 真实 LLM 金标：1 张从零卡 + 1 张改卡（后置）

---

## 11. 成功标准

### 产品成功

- 新用户不打开 ST 也能做出可玩基础卡
- 老用户能把现有 ST 卡搬进 Studio 补完
- 小说作者能把长文炼成设定包再生成卡
- 全流程最终能“一键进入 Campaign 开写”

### 工程成功

- 写卡失败不影响 Campaign 写作
- 无强制 CDN
- 关键路径有确定性测试
- 资产可版本升级（stage pack v1/v2）

### 非目标（再次强调）

- 100% 复刻玉藻前按钮行为
- 100% 复刻秋青子伪 IDE
- 承诺所有高级前端卡可视化编辑器一期完成

---

## 12. 建议的立即决策

1. **主方案锁定：原生 Card Studio（方案 3）**
2. **ABC 都做，但顺序：A/C 基础 → B 蒸馏 → C 深度/MVU**
3. **外部工具定位：规格与内容来源，不是运行时依赖**
4. **先做资产规范化与编译器，再做花活 UI**
5. **与当前 `CAMPAIGN-WORLDINFO-AND-CARD-SHELL` 并行时注意资源：壳是运行时，Studio 是生产台，别混一个 WebView 硬做**

---

## 13. 下一步（分析之后）

若认可本构想，下一步应进入正式设计规格：

1. 冻结 `CardProject` / `CardArtifacts` / Stage Pack schema
2. 画出 Phase1 界面信息架构
3. 列出从明月秋青抽取的阶段白名单
4. 写 `docs/superpowers/specs/2026-07-22-card-studio-design.md`
5. 再拆 implementation plan

在此之前不建议直接开工写业务代码。

---

## 14. 实现进度快照（2026-07-22 审阅回写）

> 详细审查见 `docs/workstreams/CARD-STUDIO-PHASE1-STATUS-2026-07-22.md`  
> 现行设计/计划：  
> - `docs/superpowers/specs/2026-07-22-card-studio-phase1-design.md`  
> - `docs/superpowers/plans/2026-07-22-card-studio-phase1.md`  
> 分支：`feat/card-studio-phase1`（worktree）

### 决策落地情况

| 建议决策（§12） | 状态 |
| --- | --- |
| 主方案原生 Card Studio | ✅ 已按此实现 |
| 顺序 A/C 基础 → B 蒸馏 → C 深度 | 🟡 A+最小 C 已做；B 仅 prefill MVP；深度 C/MVU 未做 |
| 外部工具=规格来源非运行时 | ✅ 未嵌 CDN 写卡器 |
| 先资产与编译器再花活 UI | ✅ pack 资产 + compile 优先于三栏 IDE |
| 与 worldinfo/card-shell 隔离 | ✅ 独立 worktree 分支 |

### 分期对照

| 分期 | 文档目标 | 分支现状 |
| --- | --- | --- |
| Phase 0 资产 | 知识库/阶段/文风协议入库 | 🟡 明月秋青 + distill 协议已 embed；完整知识库 JSON 未全量迁入 |
| Phase 1 A+最小 C | 从零基础卡 + 反填补强 | ✅ 基本达成（缺 GUI 金标证据） |
| Phase 2 B 蒸馏 | 切分/公式书/总结账本/断点 | ❌ 未达成；仅有摘录 prefill |
| Phase 3 深度 C/系统卡 | MVU/多定义/高级世界书 | ❌ 未开始 |

### 验收金标对照（Phase 1）

1. 20 分钟从一句话得到可导入 JSON/PNG — **工程路径具备 JSON；PNG/计时实机证据未写**  
2. 导入后 extract + 开 Campaign — **import 有 fallback 定义；自动 extract 未绑**  
3. 已有卡补世界书/开场并导出 — **C 修订+导出 ST JSON 具备；覆盖策略固定另存**

### 代码入口（便于审计）

- domain: `crates/domain/src/card_studio.rs`
- assets: `crates/domain/assets/cardstudio/{mingyue_qiuqing_v1,mingyue_distill_v1}/`
- tauri: `crates/tauri-app/src/card_studio_{api,store}.rs`
- UI: `frontend/src/components-v2/campaign/CardStudio.vue`

### 测试命令

```bash
cargo test -p storyforge-domain card_studio
cargo test -p storyforge card_studio_store
cargo check -p storyforge
```

### 下一步（实现之后，取代 §13 的“先别写代码”）

1. 补真实 LLM GUI 金标记录（A/C）
2. import 后导航/extract 体验
3. 评估 draft PR 或继续 B Phase2（外置小说文档 + 队列）
4. 与 main 上 worldinfo/card-shell 变更做合并策略

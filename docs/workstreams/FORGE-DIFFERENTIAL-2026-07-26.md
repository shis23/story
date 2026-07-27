# forge 差分预言机 RESULT（2026-07-26）

> 目标：用社区工具 ai4rpg/tavern-cards 的 forge CLI（确定性 unpack）作为
> 外部预言机，字段级验证 storyforge 导入层的世界书语义——特别是同日 landed
> 的 v3 修复（enabled 识别 / 禁用保留 / 死路由改判 / insertion_order 回退）。
> 结论：**两张验收卡差分全绿**，导入层与社区权威工具字段级一致；
> 过程中确认 2 个接受语义映射、3 个 forge 侧行为特性（非我方 bug）。

## 最终差分结果

| 卡 | 条目 (ours/forge) | 一对一配对 | 启停分歧 | 路由分歧 | 排序分歧 | 主键分歧 |
| --- | --- | --- | --- | --- | --- | --- |
| 命定之诗 | 441 / 441 | 441 | 0 | 0 | 0 | 0 |
| 卿卿 | 122 / 118 | 115 + 3 重名组对账 | 0 | 0 | 0 | 0 |

- 卿卿 122−118=4 的差**完全由重名收缩解释**：心声 ×2、战斗系统 ×3、
  长孙燕 ×2（ST 卡允许重名条目，forge manifest JSON 键唯一会吞并）。
  重名组只比数量不做字段配对（版本对应无据可依，字段比较全是假阳性）。
- 启停语义 311/311（命定）与逐条一致（卿卿一对一组）——**v3 enabled
  修复被外部预言机完全验证**。

## 接受的语义映射（论证见 `forge_differential.rs` 字段文档）

1. **forge `vectorized` ↔ ours `Selective`**（命定之诗 16 条分组标记）：
   ST vectorized = 仅向量触发；我们的 Selective 本就是向量检索池，keys 为空
   时关键词路径不触发——行为等价。原始 `vectorized` 字段保留在 entry.extra。
2. **forge `constant` ↔ ours `Both`**（卿卿 38 条）：ST (constant=true,
   selective=true) 按蓝灯常驻；我们的 `is_constant_route` 包含 Both，
   常驻注入等价，额外进向量池是超集不是缺失。

## forge 侧行为特性（差分算法为此做的适配）

1. **manifest 键保留原始空白**：实测有 `"大乾_地图事件输出\n"`（尾部换行）、
   `" 霍青棠"`（前导空格）——两侧按 trim 后名字归组。
2. **重名条目吞并**：见上。
3. **无名条目合成键 `entry_<st_id>`**：卿卿有一条无 comment 条目，forge 键为
   `entry_99`；ours 侧用 st_id 合成同款名回退匹配，命中并零字段分歧
   （反向验证了合成规则）。

## 使用方式

```bash
# 一次性准备（forge CLI 来自 github.com/ai4rpg/tavern-cards，许可为个人使用）
node <tavern-cards>/scripts/tavern-cards-forge.mjs unpack destiny \
    --file test-card.png --output <out>/destiny --fresh
node <tavern-cards>/scripts/tavern-cards-forge.mjs unpack qingqing \
    --file "卿卿 (33).png" --output <out>/qingqing --fresh

# 差分（确定性，无 LLM 无网络；未设 env 或缺卡自动跳过）
STORYFORGE_FORGE_OUT=<out> cargo test -p harness-real-llm --test forge_differential -- --nocapture
```

- 模块：`crates/harness-real-llm/src/forge_differential.rs`
  （ForgeState 解析 / 分组差分 / 脱敏报告——只含条目名、计数、枚举值，无正文）
- 测试：`tests/forge_differential.rs`（硬不变量：名字集合互覆盖 +
  一对一组零启停分歧 + 路由/排序/主键零分歧 + ≥95% 配对率）
- 单元测试 5 个覆盖：干净匹配、旧 bug 形态检出（禁用丢弃/死路由/order 丢失）、
  重名组降级、trim 归组、both↔constant 映射。

## 发现的意义与后续

- 导入层五项 v3 修复（enabled / 禁用保留 / (false,false)→Selective /
  insertion_order / depth）现在有**独立外部证据**，不只有自家单测。
- 该差分对**任何新卡**可复用：丢进一张卡 + forge unpack 一次，即可回答
  「我们的导入与社区工具是否一致」。建议新一代卡入库前跑一遍。

## V2 扩展（同日第二轮）

### V2-3 section 清单对照 —— 全绿

| 卡 | 非空开场白 | 正则（去重名集） | TH 脚本（去重名集） |
| --- | --- | --- | --- |
| 命定之诗 | 7/7 | 13/13 | 6/6 |
| 卿卿 | 6/6 | 12/12（原始 19，同名历史版本收缩） | 9/9（原始 16） |

对账中确认的 forge 行为（差分口径为此适配）：
- 正则权威计数在 `state.regex_scripts`（对象全量 13/12 条）；`正则/` 目录只有
  大体积 replaceString 外置文件（带 `replace_file`），目录数≠正则数。
- 卿卿原卡带一条**空 alternate greeting**——我们保留、forge 丢弃；对照按非空口径。
- Zod schema 引导脚本（"MVU脚本"）被 forge 特化到 `state.zod` + `schema.ts`，
  不写入 `脚本/` 目录；TH 口径 = 目录数 + zod。

### V2-2 MVU schema 对照（InitVar YAML 作 ground truth）—— 评测板

以 forge 解包出的 `[InitVar]` 变量初始树为作者定义 ground truth
（命定之诗 55 叶 / 卿卿 265 叶），给翻译产物 variable_schema 打分：

| 组合 | 覆盖率 | 幻觉率 | 备注 |
| --- | --- | --- | --- |
| 命定之诗 × flash | 50.9% | 22.2% | |
| 命定之诗 × pro | 50.9% | **12.5%** | 同覆盖下精度更高 |
| 卿卿 × flash | **100%** | **0%** | 模板化 schema（见下），此前按键数被低估 |
| 卿卿 × pro | ≥17.4%（下界） | 0% | 旧证据只存 40/141 键样本；上限已放开到 512，下轮验收出全量 |

**关键发现：schema 键记法三态不稳定**（评测器已归一化，各态实测均来自真产物）：
1. 点记法 `stat_data.主角.属性.力量`（flash 命定）
2. 斜杠记法 `/主角/属性/力量`（pro 命定）——归一化前被误判 100% 幻觉
3. 模板记法 `女性角色.{角色名}.好感度`（flash 卿卿）——一条模板代表所有
   同构角色子树，展开后语义覆盖 100%

**给翻译主线的跟进项**：下游消费端（meta_apply_mvu_schema / ui_bindings /
变量读写）目前只认字面键——需要在 mvu_import 解析层做记法规范化
（斜杠→点；模板键需决定展开或保留参数化语义）。另：命定之诗两模型
覆盖率都停在 50.9%，未覆盖的 45 叶集中在哪些子树值得查一次
（怀疑与分析器 InitVar 预算截断有关）。

### 跟进落地（2026-07-27）

两条跟进项当天闭环：

1. **键记法规范化已入库**（commit 92d7677）：canonical = 点记法、无
   `stat_data.` 前缀、模板段 `{...}`。`normalize_mvu_key` /
   `normalize_schema_keys`（domain）在 mvu_import 解析层三处
   （variable_schema / ui_bindings / interactions，含键自引用表达式
   `rewrite_self_ref_expr`）+ `meta_apply_mvu_schema` 应用边界（存量兜底）
   收敛；分析器提示词同步钉死记法（原提示词示例带 `stat_data.` 前缀与
   「key 对齐 P1」自相矛盾，正是漂移源之一）。前端 `utils/mvuKey.js` 镜像，
   取值/交互计划跨记法匹配，写回优先已存储键。模板键决策：**保留参数化
   语义**（占位符段统一 `{}` 记法，评测端已按通配对齐；运行时展开留给
   实例绑定工程）。
2. **50.9% 覆盖率缺口根因坐实 = InitVar 预算确定性截断**（commit 8296fa6）：
   两模型分数完全相同不是巧合——`build_worldbook_variable_section` 旧预算
   单条 4000 字/总 14000 字，命定之诗 [InitVar] 树超预算被 truncate 掉中段
   （truncate_for_prompt 保头 4/5 尾 1/5），中段子树对任何模型都不可见。
   修复：[InitVar] 数据条目提额单条 24K/总 40K（规则类维持 4K），
   中段哨兵回归测试锁死。修复后重测见下节。

### 修复后重测（2026-07-27，2 卡 × 2 模型真验收全绿 353s + 差分重跑）

| 组合 | 覆盖率 | 幻觉率 | 修复前 |
| --- | --- | --- | --- |
| 命定之诗 × flash | **67.3%**（37/55 叶） | 14.5% | 50.9% / 22.2% |
| 命定之诗 × pro | **65.5%**（36/55 叶） | 20.0% | 50.9% / 12.5%（40 键样本） |
| 卿卿 × flash | **100%** | **0%** | 100% / 0% |
| 卿卿 × pro | **100%**（全量 32 模板键） | **0%** | ≥17.4% 下界（样本上限 40） |

判读：
- **根因验证成立**：截断消失后两模型 destiny 分数不再相同
  （50.9/50.9 → 67.3/65.5）——模型看到全树后行为自然分化，
  此前的"完全一致"确系输入侧确定性截断。
- 卿卿 pro 的样本下界问题随 512 键上限解除：两模型都收敛到
  模板化 schema（32 键，占位符段语义覆盖 265 作者叶）。
- destiny 剩余 ~33% 缺口与 14-20% "幻觉"进入真模型行为区间：
  被标幻觉的键（世界.新闻 / 事件.莉莉.* 等）部分疑似来自开场白
  UpdateVariable 种子与 JS（InitVar 树之外的合法变量源）——
  ground truth 只含 [InitVar]，这是评测口径的已知边界，不是缺陷。
- worldbook / section / 正文三重差分对新证据依旧全绿。

### V2-1 正文级对照 —— 全绿，导入保真闭环

一对一配对条目的正文与 forge 外置文件逐条等值比较（归一化：CRLF→LF +
首尾空白；报告只含名字/长度，无正文）：

| 卡 | 比较 | 一致 | 缺失 | 分歧 |
| --- | --- | --- | --- | --- |
| 命定之诗 | 425 | **425** | 0 | 0 |
| 卿卿 | 110 | **110** | 0 | 0 |

- 未参与比较的条目均有账：manifest `path` 为空串 = 空内容条目
  （分组标记等，forge 不落文件）；重名组 7 条跳过（无配对依据）。
- 至此导入层对社区预言机实现**结构（V1）+ 清单（V2-3）+ 内容（V2-1）
  三重全绿**——「结构对了但内容烂了」的最后一类风险收口。

## 2026-07-27 口径扩展：开场白 UpdateVariable 种子入作者树（#21）

此前作者树只取 InitVar YAML。`extract_update_variable_seed_paths` +
`collect_opening_seed_paths` 把作者在开场白 `<UpdateVariable><JSONPatch>`
里就地初始化的变量并入 ground truth 后重算。种子 path 归一：斜杠→点、
尾段 `-`（JSON Patch 数组追加记号）丢弃、空段清理；非法块静默跳过。

**真实重算运行（2026-07-27）**：用历史会话已生成的 forge unpack 产物
（确定性解析，无 LLM、无网络）+ `artifacts/card-translation/` 四份证据
重跑 `forge_schema_alignment_scores_translations`，全绿。各组合作者树
规模由测试 stdout 直接报出（"InitVar X 叶 + 开场白种子 Y（并集 Z）"）。

| 组合 | 作者树（并集） | 翻译键 | 有据 | 幻觉率¹ | 覆盖率 | 旧覆盖率 / 旧幻觉率（仅 InitVar） |
| --- | --- | --- | --- | --- | --- | --- |
| 命定之诗 × flash | 56（InitVar 55 + 种子 22，净增 1） | 69 | 59 | **14.49%** | 67.86% | 67.3% / 14.5%（作者树 55） |
| 命定之诗 × pro | 56（同上） | 45 | 36 | **20.0%** | 66.07% | 65.5% / 20.0%（作者树 55） |
| 卿卿 × flash | 265（InitVar 265 + 种子 29，**净增 0**） | 32 | 32 | **0.0%** | 100% | 100% / 0%（无变化） |
| 卿卿 × pro | 265（同上） | 32 | 32 | **0.0%** | 100% | 100% / 0%（无变化） |

判读（已据真实重算修正）：

- **种子并入只影响命定之诗**：22 条种子里 21 条与 InitVar 重叠，
  净增 1 个作者键，作者树 55→56，覆盖率因此从 67.3%→67.86%、
  65.5%→66.07%（分母变大）。
- **卿卿种子 29 条全部已在 InitVar 树内**（并集 265 = InitVar 265，
  净增 0），种子并入对卿卿分数**无任何影响**——卿卿本就是 0% 幻觉
  （见上一节"修复后重测"表），并非"此前误判的幻觉被种子消除"。
  （本节早先版本的判读把卿卿描述为"此前的幻觉全是开场白种子"，
  与真实重算矛盾，特此更正。）
- **命定之诗仍有当前口径未对齐键**：flash 10 个、pro 9 个，二者并集
  10 个：`世界.新闻`、`世界.酒馆留言板`、`世界.午后茶会`、`主角.装备`、
  `命定系统.核心机制`、`事件.信号`、
  `事件.莉莉.{阶段,侵蚀度,已净化区域,净化次数}`。它们确实不在
  InitVar 或开场白种子里，但**不能据此认定为模型虚构**：完整 forge
  产物显示 `事件.莉莉.*`、`事件.信号` 由作者世界书 JS 初始化，
  `主角.装备` 也被作者脚本读取。当前作者树尚未收集世界书 JS/EJS 中的
  `getMessageVar` / `setMessageVar` 变量源。因此表中的 14.49-20% 只是
  “相对 InitVar ∪ 开场白种子未对齐率”（真实幻觉率的上界），不是已经
  坐实的模型虚构率。开场白种子口径扩展本身已完成；JS/EJS 变量源是后续
  评测口径边界。

¹ 字段名沿用评测器 `hallucination_pct`；在当前 ground truth 未覆盖全部作者
变量源的情况下，应读作“未对齐率”。

种子解析形态由单元测试锁定（`test_extract_update_variable_seed_paths`）：
JSONPatch 数组、`/-` 追加记号尾段丢弃、非法块静默跳过。

### 复现命令

```bash
# 前置：forge unpack 产物目录（历史会话已生成；forge CLI 来自
# github.com/ai4rpg/tavern-cards，unpack 是确定性本地解析，不调 LLM 不联网）
# 证据目录：artifacts/card-translation/（4 份 JSON，含 stats.schema_keys_sample）

STORYFORGE_FORGE_OUT=<forge-out 目录，含 destiny/ 与 qingqing/> \
STORYFORGE_CT_EVIDENCE_DIR=<artifacts/card-translation 绝对路径> \
cargo test -p harness-real-llm --test forge_differential \
    forge_schema_alignment_scores_translations -- --ignored --nocapture
```

该测试标记为 Rust `#[ignore]`：普通 `cargo test` 会明确报告 `ignored`，不会
把未执行伪装成 `ok`。显式加 `--ignored` 后，两个 env 缺失会直接失败；本轮
重算已显式设定两个 env，确保真实执行。

### 输入指纹（防临时目录丢失后无法验明）

- `test-card.png` SHA-256：
  `20bd48496917b8474a2183c3c6cd56f7fa9b7d3704bdd100d704045048483a1b`
- `卿卿 (33).png` SHA-256：
  `d8397338ffdb9641afd99545fcd43f6e6eb55a7be749aad70748c6b61f4aef48`
- Forge CLI：`https://github.com/ai4rpg/tavern-cards`，commit
  `4a565bbe04b723405a1d763923acb9b15a56faa6`
- 本轮 unpack 树指纹（每个文件先取 SHA-256，按相对路径排序，以
  `<hash><两个空格><relative-path>` UTF-8、LF、无末尾换行拼接后再取 SHA-256）：
  - `destiny/`：454 文件，2,874,598 bytes，
    `764711fe2df73be15d4bd7ed46368b8a978eb8f03c0f3806d18ba9daf77a6bb0`
  - `qingqing/`：142 文件，3,391,670 bytes，
    `0cedf549da36135facfb87a02b522b422018b32a2bed7f78886b3519b60e4d79`

源卡位于仓库根目录（被 `.gitignore` 排除）；即使历史 `%TEMP%` 解包目录被清理，
仍可用上述固定 CLI commit 重新 unpack，并用树指纹核验输入一致性。

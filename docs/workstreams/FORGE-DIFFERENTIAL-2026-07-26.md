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
- v2 方向（未做）：
  1. 内容级对照——forge 把正文写成 yaml/txt 文件，归一化后可做 per-entry hash；
  2. schema 对齐——forge 从卿卿抽出了社区标准 `schema.ts`（Zod），可与
     MVU 翻译产物的 variable_schema 键集做对照（证据文件已含 keys）；
  3. 开场白/正则/脚本三个 section 的清单对照（forge 已解包出目录）。

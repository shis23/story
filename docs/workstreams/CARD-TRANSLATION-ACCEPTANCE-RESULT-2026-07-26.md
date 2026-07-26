# 新型卡片翻译验收 RESULT（2026-07-26）

> 目标：命定之诗（`test-card.png`，引导器+远程应用形态）与 卿卿（`卿卿 (33).png`，
> 单体全内嵌 chara_card_v3）两张卡，经 Meta Agent 在 harness 内完整翻译为本项目格式
> （角色抽取 + MVU 五合一 + 世界书 + 组件兼容归类 + 格式校验），
> 使用 `https://cli.2529985.xyz/v1` 的 deepseek-v4-pro / deepseek-v4-flash
> （`reasoning_effort=max`）双模型验收。
> 结论：**第 5 轮 4/4 组合全绿**（`test result: ok`，总墙钟 395 秒）。

## 最终验收结果（第 5 轮，全部并行）

| 组合 | 角色定义 | schema 字段 | 更新规则 | 置信度 | 路由 | 墙钟 |
| --- | --- | --- | --- | --- | --- | --- |
| 命定之诗 × pro | 19 | 32 | 10 | 0.72 | Hybrid | 159s |
| 命定之诗 × flash | 5 | 36 | 19 | 0.55 | Hybrid | 134s |
| 卿卿 × pro | 17 | 141 | 44 | 0.65 | Hybrid | 261s |
| 卿卿 × flash | 15 | 32 | 35 | 0.60 | Hybrid | 158s |

- 每组合 16 项确定性断言全过：导入（条目数 / 禁用保留 / 无死路由 / 禁用不进常驻）、
  组件归类零漏项（命定 13+6、卿卿 19+16）、抽取（定义数 / persona 非空 / 期望角色名命中，
  卿卿 6/6 名字全中过）、翻译（非降级空壳 / schema 来源于卡的关键词证据 / 规则数 / 路由合法）。
- schema 质量抽样：命定之诗还原 `stat_data.主角.属性.力量`、`登神长阶.权能`、
  `命定系统.命运点数`、`事件.莉莉.侵蚀度` 等完整层级路径；卿卿×pro 还原整棵
  女性角色好感度/在场树（141 字段）。
- 脱敏证据（计数/键名/断言，无正文）：`artifacts/card-translation/*.json` 4 份。
- 入口：`crates/harness-real-llm/tests/card_translation_acceptance.rs`
  （确定性部分无凭证即可跑；真实部分 `#[ignore]`，env 见文件头注释；key 仅从环境读取）。

## 五轮迭代与根因链（每轮失败都定位到具体根因）

| 轮 | 形态 | 结果 | 根因与修复 |
| --- | --- | --- | --- |
| 1 | 串行 | 0/4 | 未设 max_tokens（max 推理吃光补全预算→JSON 截断→静默空壳）；pro 被 Cloudflare 边缘 ~100s 掐成 524；tracing 无 subscriber 根因不可见 |
| 2 | 串行 | 命定×pro 16/16 后被主动终止 | max_tokens=16384 + 缩 prompt 生效；用户要求切并行 |
| 3 | 8 路并行 | 0/4，11.4 分钟 | 抽取全成、分析全空：**MVU 分析器漏配 terminal_tools**（模型早期成功提交后循环空转 8 轮丢产物）；8 路并发打空中继号池（1101/auth_unavailable） |
| 4 | 并行×2 + 终止工具 + 退避 | 3/4 | 终止工具修复生效（命定×pro 685s→167s）；卿卿×pro 仍 524；新抓到工具参数尾随垃圾解析失败 |
| 5 | + 流式工具循环 + 参数容错 | **4/4** | SSE 首字节早到规避边缘超时；括号配平兜底参数解析 |

## 代码变更（除 harness 外均为生产路径修复/增强，各带回归测试）

**导入层（任务 2）**
- `domain/world_info.rs`：禁用条目**保留**并标 `disabled=true`（原为丢弃）——MVU 卡的
  [InitVar]/DLC 靠禁用条目当数据，card-shell 世界书开关也需要在册；注入路径均已检查
  disabled。v3 `enabled:false` 从 extra 识别；`insertion_order`/`extensions.depth` 回退；
  `(constant=false, selective=false)` 由 Disabled 改判 Selective（ST 语义：仍按主键触发）。
- `domain/character.rs`：`null_to_default` 容忍野生卡的 `"tags": null` 等显式 null。
- `infra-import`：PNG 无 `chara` 块时回退 `ccv3` 块（v3-only 卡）；compat 断言更新为
  「禁用条目保留且不进常驻注入」。
- `tauri-app/lib.rs`（仅两行守卫）：per-card 注入存储与向量入库跳过禁用条目（保持旧行为）。

**翻译层（任务 3）**
- `app-meta/prompts/mvu_analyzer.rs`：输入面扩宽——世界书变量条目（含禁用态 [InitVar]）、
  EJS 控制器（在场/分阶段门控，要求翻译为 schema+规则而非保留源码）、tavern_helper
  清单（同名多版本按启用去重）、regex 界面摘要（元数据不含巨型正文）、开场白
  `<UpdateVariable>` 种子；系统提示词补新一代卡形态知识；
  **`terminal_tools: ["emit_mvu_translation"]`（生产 bug 修复）**。
- `domain/mvu_translation.rs`：复杂度打分纳入启用 TH 脚本（修 TH-only 卡误判
  PureData 短路跳过 LLM）。
- `app-meta/mvu_import.rs`：**流式工具循环**（规避边缘超时，慢供应商通用收益）；
  第 1 层解析对工具参数尾随垃圾做括号配平兜底。
- `app-agent/character_extractor.rs` + prompts：流式工具循环；世界书输入
  启用优先 + 45K 总量 / 900 每条截断。
- `infra-llm/http_client.rs`：`STORYFORGE_LLM_TIMEOUT_SECS` 覆盖同步请求超时（默认 120s 不变）。

**harness（任务 4）**
- `harness-real-llm/src/card_translation.rs`（新）：期望值/检查/组件归类/翻译流程
  （带退避重试、空壳判定）/脱敏证据。
- `tests/card_translation_acceptance.rs`（新）：确定性验收（卡在本机即跑，缺卡跳过）
  + 真实验收（组合级并行，信号量限 2，组合内抽取∥分析双路并发）。

## 测试

- `storyforge-domain` 258 / `storyforge-app-meta` 118 / `storyforge-app-agent` 106 /
  `storyforge-infra-import` 55 全绿（含本次新增：v3 enabled 保留、死路由修复、
  insertion_order/depth 回退、ccv3 导入、TH-only 非 PureData、分析器新区块、
  工具参数尾随垃圾容错）。
- workspace 全量回归：见文末「验证」。

## 剩余风险与后续建议

1. ~~**翻译产物消费端仍断**~~ **已于同日接通**（见文末「消费端接线补记」）：
  `update_rules` 注入后处理提示词、`ui_bindings` 原生状态面板、`interactions`
  原生分发器均已落地。残余：`run_original_js` 仍留桩；value_expr 只做保守解释
  （增量/字面量/裸词，JS 表达式拒绝）；SQLite 后端在 MVU 迁移前跳过规则收集
  （与 fallback_fragments 同门）。
2. flash 在命定之诗上的抽取数波动（3-10 个定义），下限断言按 1 收口；
  语料级稳定性评估待更多卡样本。
3. 中继并发安全水位实测为 4 路（8 路打空号池）；harness 已用信号量限 2 组合。
4. `reasoning_effort=max` 为中继透传参数，DeepSeek 侧实际力度未验证（接受且返回
  reasoning_content）。
5. 证据含角色名与 schema 键名（无正文）；对外分享前自查。
6. 同日旁路评审 `CARD-SHELL-REVIEW-2026-07-26.md`：宿主线对同两张卡的 UI 保真
  有 17 条确认缺口（内联壳不执行 / 卿卿开场不识别 / CSP 缺失 / var_write 越权等），
  与本验收互证「翻译为主干、沙箱做表现层且先补墙」。
7. 参考对照（未执行）：ai4rpg/tavern-cards 的 forge CLI 可作导入差分预言机、
  其 `state.ts`/MVU 规范可作 schema 产物的社区对齐目标（许可为个人使用，注意边界）。

## 终审（code-reviewer 代理，2026-07-26）

- **A 回归检查（禁用条目保留 / 路由改判）：清**——所有注入消费方均经
  `is_constant_route`/`is_selective_route` 门（内含 `!disabled`）；仅有的两处裸遍历
  正是本次在 tauri-app 加的两行守卫；分析器/抽取器读禁用条目属设计意图（只进
  LLM 分析输入，不进故事注入）。
- **B 流式切换：清**——terminal_tools/推理捕获/取消语义与非流式一致；progress
  通道按值移入、返回即关闭，排水任务随之退出，无泄漏无死锁。
- **C 解析回退：RISK（已修）**——截断参数 + 完整小嵌套对象可被误抓成
  「合法但近空」翻译（confidence 0.5 绕过空壳判定）。已按建议收紧：候选对象必须
  携带五合一顶层键（或包装键）才接受，附截断回归测试
  （`truncated_tool_args_fail_instead_of_yielding_near_empty_translation`）。
- **D null_to_default 往返：清**——仅影响反序列化，`null → []` 为良性归一化。
- **E harness 质量：清**——证据只含计数/键名/角色名/断言；仓库无硬编码密钥
  （key 仅环境变量；全仓 grep 无泄漏）。
- 遗留 NIT（不阻塞）：`set_enabled` 的 Disabled-route 恢复分支因 default_route
  改判成为死代码，待清理；greeting 扫描有一处冗余 re-scan。
- **判定：ship**（fix-first 项已修复并测试锁定）。

## 验证

- 真实验收：`cargo test -p harness-real-llm --test card_translation_acceptance -- --ignored`
  于 2026-07-26 第 5 轮 `ok`（4/4 组合、64 项断言、4 份证据）。
- workspace 全量：`cargo test --workspace` 于 2026-07-26 全绿——38 个测试套件
  0 失败、共 800 个测试通过（工作树含 card-shell WIP 的状态下）。

## 消费端接线补记（2026-07-26 同日，验收后追加）

翻译闭环的「下一半」：产物从 CampaignStore 走进运行时。

**1. `update_rules` → 后处理提示词（Rust）**
- `app-agent/prompts/postprocess.rs`：`build_postprocess_user_msg_with_context`
  新增【卡片变量更新规则】区块（去重 + 预算 80 条 / 单条 400 字 / 总 12K，
  截断附说明），插在【本轮成文】之前；系统提示词补规则对照说明
  （成文触发才执行、冲突以成文为准）。
- 参数线：`run_postprocess` → `run_postprocess_pipeline_with_prompt` →
  `PipelineOrchestrator::run_postprocess` 全链增 `mvu_update_rules: &[String]`；
  旧入口/无规则调用传 `&[]`，输出与旧版字节级一致（有回归断言）。
- `tauri-app/lib.rs`：`collect_mvu_update_rules`（def→source 查找链，
  **按源卡去重**——同卡多实例只贡献一次规则）+ `_for_backend` SQLite 门；
  start_writing / regenerate 两个后处理调用点接入。
- 测试：app-agent 123 / app-pipeline 89 / tauri-app 294 全绿（含捕获客户端
  验证规则块确实进入 LLM 请求）。

**2. `ui_bindings` → 写作面原生状态面板（前端）**
- 既有 `MvuStatusBar.vue`（四种 BindingDisplay 原生渲染）此前只挂管理页；
  新增 `buildCampaignMvuStatusSections`（campaign 变量打底 + 实例变量覆盖，
  每个卡绑定实例一节）+ `useMvuStatusPanel` composable
  （getCard→metaGetMvuTranslation→listInstances+getCampaignVariables，
  token 竞态守卫）+ `MvuStatusPanel.vue`，挂 WritingScreen 预留的
  `after-messages` 槽（"当前状态紧随最后一条剧情"，此前空置）。
- 刷新时机：campaign 切换；postprocess running→done（后处理写回变量后拉新值）。

**3. `interactions` → 原生分发器（前端）**
- `utils/mvuInteractions.js`：动作扁平化（multi 递归、深度上限 5）、
  `value_expr` 保守解释（`+N` 增量 / `-N` 数值增量否则字面量 / JSON 字面量 /
  裸词字符串；含 JS 标记的表达式拒绝并记 skipped）、`planMvuInteraction` 纯计划。
- 分发：`modify_variable` 经 `persistShellVariableWrite` 落盘（恰好一个卡绑定
  实例 → `instance:<id>:<key>` 实例作用域，否则 campaign 作用域），写完刷新面板；
  `trigger_next_turn` 的 hint 走 `startWriting`（写作中跳过）；
  `run_original_js` 留桩记 skipped（与 domain 注释一致）。
- 面板渲染交互按钮（busy/写作中禁用），AppV2 经 `#after-messages` 槽接线。
- 测试：node --test 371→379、vitest 50→52，vite build 通过。

**接线终审（code-reviewer，同日）**：Rust 参数线全调用点 / 收集器边界 /
预算截断 / 前端竞态 token / postprocess_skipped 沿触发 / AppV2 接线顺序
全部核实通过；2 个 HIGH 已修——
1. `value_expr` 规范格式 `"hp - 10"`（分析器一号示例）曾被误判为字符串字面量
  静默腐蚀数值变量 → 新增 keyed-delta 分支（前导标识符必须等于目标 key）+
  算术混排拒绝落字面量，回归测试锁定；
2. 交互写入与壳写入的「单实例」判据不一致（卡绑定实例数 vs 全 campaign
  实例数）可致同键分裂作用域 → 两条路径判据统一为
  「卡绑定实例优先，全 campaign 单实例回退」（含复核指出的
  零渲染节回退不对称 LOW，也已对齐并有测试）。
- 复核判定 **SHIP**（评审代理独立重放验证：判据代码 + 测试 + build）。
- workspace 全量回归（含本次接线）：70 个测试目标全 `ok`，0 失败；
  前端最终 node --test 380 / vitest 52 全过。

## 异机复现指南（2026-07-27 补，发布门禁 P1）

验收分两层，确定性层任何机器可跑，真实模型层需要凭证与验收卡：

```powershell
# 确定性层（无凭证；验收卡缺失时逐卡跳过、不 fail——CI 上属正常）
cargo test -p harness-real-llm --test card_translation_acceptance

# 真实模型层（#[ignore]）
$env:LLM_BASE_URL='https://cli.2529985.xyz/v1'
$env:LLM_API_KEY='<key，只进环境变量，绝不写入文件>'
$env:LLM_MODEL='deepseek-v4-pro'
$env:STORYFORGE_CT_MODELS='deepseek-v4-pro,deepseek-v4-flash'
$env:STORYFORGE_LLM_TIMEOUT_SECS='600'
cargo test -p harness-real-llm --test card_translation_acceptance -- --ignored --nocapture
```

环境变量总表：

| 变量 | 作用 | 默认 |
| --- | --- | --- |
| `STORYFORGE_CT_CARD_DESTINY` | 命定之诗卡 PNG 路径 | 仓库根 `test-card.png`（未入库的本机文件） |
| `STORYFORGE_CT_CARD_QINGQING` | 卿卿卡 PNG 路径 | 仓库根 `卿卿 (33).png`（未入库） |
| `STORYFORGE_CT_MODELS` | 逗号分隔的验收模型列表 | 必填（真实层） |
| `STORYFORGE_CT_EVIDENCE_DIR` | 证据 JSON 输出目录，**必须绝对路径**（集成测试 cwd 在包目录，相对路径会写错地方） | `<repo>/artifacts/card-translation` |
| `LLM_BASE_URL` / `LLM_API_KEY` / `LLM_MODEL` | 中继凭证与 require_real_llm 兜底模型 | 必填（真实层） |
| `STORYFORGE_LLM_TIMEOUT_SECS` | 单请求超时（Cloudflare 边缘 ~100s 掐 524，流式已规避，仍建议 600） | 客户端默认 |

异机所需材料：两张验收卡 PNG（个人素材，不入库，路径经上表 env 指入）+
中继 key。证据 JSON 为脱敏格式（计数/键名/断言，无卡正文），可直接入库对比。

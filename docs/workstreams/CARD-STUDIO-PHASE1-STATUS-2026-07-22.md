# Card Studio Phase 1 Status & Review (2026-07-22)

> Branch / worktree: `feat/card-studio-phase1` (`.worktrees/feat-card-studio-phase1`)  
> HEAD (at review): `fb8b6dd`  
> Scope reality: Phase1 A 完整 + C 最小 + B MVP 骨架（不是完整小说蒸馏流水线）

---

## 1. 审阅结论（先看这个）

### 做得对的

1. **架构边界正确**：写卡是 Card Studio 生产台，不劫持 Director/Editor/Campaign 写作链。
2. **外部工具定位正确**：玉藻前 / 秋青子 CDN 不进主路径；明月秋青方法论与文风工具被 **native 化**为 stage pack / distill pack。
3. **A 路径可用闭环**：项目 → 阶段 LLM/手工 → 规则+方法论审查 → 编译导入 CharacterStore + CharacterCard。
4. **C 路径默认另存**：`reverse_parse` 后编译/import 生成新 domain id + 新 store id + 新 playable card，不覆盖原卡。
5. **提示词不是空骨架**：`mingyue_qiuqing_v1` 嵌入 creative principles / absolute zero / tag_spec / worldbook / stage templates / self-check。
6. **性格默认手写优先**：`allow_ai_freewrite=false`，UI 显式开关。
7. **测试有确定性覆盖**：domain 20 + store 2；prompt 装配、checks、compile、reverse-parse、novel excerpt 均有单测。

### 主要缺口 / 风险

| 级别 | 问题 | 说明 |
| --- | --- | --- |
| High | **无真实 LLM golden-path 证据** | 单测不调用模型；“20 分钟做出可玩卡”未在 GUI 实跑验收 |
| High | **B 远未达到可行性文档 Phase2** | 现为摘录 prefill，不是整本切分 + 小/大/超级总结 + 断点续跑 |
| Med | **导入后卡片体验仍粗糙** | import 只 seed fallback 单主角定义；未自动 extract；PNG 导出未接 Studio |
| Med | **同名卡 tool_ctx 语义偏弱** | 已改为按 id 去重，但 Campaign 开档仍可能依赖 extract 质量 |
| Med | **大小说存储是折中** | >8 万字丢全文只留摘录；无法“重读未抽样章节”再蒸馏 |
| Med | **前端信息架构仍是单栏 MVP** | 无左阶段/中对话/右 diff 三栏；无项目 diff、无字段级变更对比 |
| Med | **schema gen 脏文件未提交** | `crates/tauri-app/gen/schemas/*.json` 常被改动，分支上未纳入提交 |
| Low | **C 反解析深度有限** | 只反填基础字段+世界书；MVU/EJS/多定义/扩展字段不进 artifacts |
| Low | **无 API 层集成测试** | create_from_character / prefill / import 的 tauri command 级测试缺失 |
| Low | **署名/授权策略文档未落地** | 资产注明来源“三明月/明月秋青”，但产品内署名策略未产品化 |

### 总体评级

- **工程方向**：A（正确）
- **A 路径完成度**：约 **80%**（差 GUI 金标与导出 PNG）
- **C 路径完成度**：约 **60%**（最小修订+另存可用；深度补洞/diff/覆盖策略未做）
- **B 路径完成度**：约 **25%**（预填骨架；非完整蒸馏）
- **是否可合 main**：建议 **先在 worktree 继续实机验收**，或开 draft PR 但不宣称 Phase2 完成。

---

## 2. 交付清单（相对可行性文档）

| 目标 | 状态 | 证据 |
| --- | --- | --- |
| 原生 Card Studio，不嵌 ST 写卡 iframe | ✅ | `CardStudio.vue` + `cardstudio_*` |
| A 从零 brief→基础卡 import | ✅ | stages + import_compiled |
| 明月秋青方法论 pack | ✅ | `assets/cardstudio/mingyue_qiuqing_v1/**` |
| 规则检查 + 方法论审查 | ✅ | `run_checks` / `run_review` / merge demote |
| C reverse_parse + 卡库入口 | ✅ | `create_from_character` + 写卡工作室修订 |
| C 默认另存 | ✅ | compile 新 id；store `save` 新 id |
| B 完整蒸馏流水线 | ❌ | 仅 excerpt prefill |
| B 文档入库/断点/进度 | ❌ | 无 |
| MVU / 多角色定义生成 | ❌ | 仅 fallback 单定义 |
| 前端美化写卡 | ❌ | 明确 non-goal |

---

## 3. 实际代码地图

### Domain

- `crates/domain/src/card_studio.rs`
  - modes: `FromScratch` | `FromNovel` | `FromExistingCard`
  - stages: brief → basic → personality → worldview → opening → review → compile_import
  - packs: `mingyue_qiuqing_v1`, `mingyue_distill_v1`
  - key APIs: stage prompt / review prompt / checks / merge / compile / reverse_parse / novel sample+prefill

### Tauri

- `crates/tauri-app/src/card_studio_store.rs` → `data/card_projects.json`
- `crates/tauri-app/src/card_studio_api.rs` commands:
  - list / create / create_from_novel / create_from_character
  - get / delete / update_artifacts / set_stage / set_options
  - run_checks / run_review / run_stage / complete_manual_stage
  - prefill_from_novel / compile / import_compiled / list_stages

### Frontend

- `CardLibrary.vue`：写卡工作室入口 + 详情「写卡工作室修订」
- `CampaignPanel.vue`：`cardsView` library|studio + `studioSeed`
- `CardStudio.vue`：A/B/C 创建、阶段轨点击切换、生成/审查/导出 JSON/导入/删除

---

## 4. 提交历史（branch vs main）

```text
cabaace feat(cardstudio): Phase 1 from-scratch Card Studio MVP
ef71f38 feat(cardstudio): embed Mingyue Qiuqing stage pack prompts
33c5606 feat(cardstudio): methodology review with tag and worldbook checks
114ecae feat(cardstudio): revise existing cards via reverse-parse (C path)
d2ce5bc feat(cardstudio): novel adapt prefill path (B MVP)
6746af3 chore(cardstudio): track style_notes in draft and ignore tmp extracts
fb8b6dd feat(cardstudio): delete/export usability + slim large novel storage
```

---

## 5. 测试证据

```text
cargo test -p storyforge-domain card_studio
# 20 passed

cargo test -p storyforge card_studio_store
# 2 passed

cargo check -p storyforge
# ok
```

**未做**

- 真实 LLM 端到端 1 张 A 卡 + 1 张 C 卡
- 导入后 Campaign 开档 GUI 路径
- export PNG round-trip
- command 级 mock LLM 集成测

---

## 6. 产品行为契约（当前实现）

### A 从零

1. 卡库 → 写卡工作室 → 填项目名/brief → 从零创建
2. 确认意图 → AI 生成本阶段（basic/personality/worldview/opening）
3. 规则检查 / 方法论审查
4. 编译并导入卡库（另存新卡）
5. 可导出 ST JSON（不强制导入）

### B 小说（MVP）

1. 粘贴正文（硬上限约 40 万字）
2. 自动头/中/尾摘录；>8 万字不落全文
3. AI 预填 → 产物可手改 / 阶段重跑
4. 检查 → 导入/导出

**不是**：整本 5 万字段落报告 → 阶段公式书 → 超级总结状态机。

### C 修订

1. 卡详情 → 写卡工作室修订
2. reverse_parse → 落检查阶段
3. 可点阶段局部重跑
4. 编译并另存为新卡（原卡保留）

---

## 7. 设计债与下一步建议

### 立刻可做（低风险）

1. 实机 golden path 记录（A/C 各一张）写入本目录 RESULT
2. 提交或 regenerate 清理 `gen/schemas` 脏文件策略
3. import 成功后一键「识别角色 / 去卡库」导航
4. Studio 导出 PNG 复用 `export_st_card_png`（导入后）

### 中期（B 真·Phase2）

1. 小说外置文档表（不塞 `card_projects.json`）
2. 切分队列 + 进度 + 断点 `project_state`
3. 总结账本 → 多角色候选 → 选主卡再 prefill
4. 文风 final_prompt 可选挂写作 profile

### 明确不做（继续坚持）

- 默认托管玉藻前/秋青子远程脚本
- 写卡阶段污染 Campaign active preset
- 一期承诺完整 MVU/EJS/前端美化 IDE

---

## 8. 审查人备注

本轮实现“能跑的原生写卡台”目标基本达成；相对最初可行性分析，**A 超预期补强了方法论 pack 与审查，C 达到最小另存修订，B 仅探路**。文档此前仍停在 Phase1 设计骨架，与代码漂移明显——应以本 STATUS 与更新后的 design/plan 为准。

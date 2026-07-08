# 角色识别计划（Character Extraction）

> 状态：已实现核心能力（2026-06-15 完成），两阶段流水线 + evidence/confidence + 用户审阅为后续增强计划（未实现）
> 目标读者：可交给小模型按阶段执行
> 关联：`docs/AGENT_INTERFACES.md` §6.2、`docs/DOCS-CODE-AUDIT.md`、`docs/PLAN-ST-IMPORT-EXPORT.md`

## 目标

导入 ST 角色卡时，自动识别卡内包含的多个角色，为每个角色生成 `CharacterDefinition`，支撑 Campaign 多角色写作。

## 非目标

- 不做运行时动态角色识别（识别只在导入时跑一次）。
- 不在角色识别计划内设计导出回 ST 的格式；ST 兼容导出由 `PLAN-ST-IMPORT-EXPORT.md` 跟踪。

## 当前事实（已实现）

### 核心代码

| 文件 | 职责 |
|------|------|
| `crates/app-agent/src/character_extractor.rs` | 编排入口 `extract_characters()` + 5 层兜底解析 `parse_character_definitions_from_response()` + `attach_definitions_to_card()` |
| `crates/app-agent/src/prompts/character_extractor.rs` | `CHARACTER_EXTRACTOR_SYSTEM_PROMPT` + `make_character_extractor_config()` + `build_character_extractor_user_msg()` + `register_character_extractor_tools()` |

### 已实现能力

1. **`extract_characters(runtime, character, mvu_schema, cancel)`** — 调 `AgentRuntime::run_tool_loop` 跑识别 Agent，产出 `Vec<CharacterDefinition>`。
2. **`parse_character_definitions_from_response(resp)`** — 5 层兜底解析 LLM 输出：
   - 层 1：`emit_characters` 工具调用（arguments JSON）
   - 层 2：整个 content 是 JSON 数组
   - 层 3：` ```json ` 代码块
   - 层 4：裸代码块
   - 层 5：手写括号配平（`[...{...},{...}...]` 逐对象解析）
3. **`attach_definitions_to_card(definitions, card_id)`** — 回填 `card_id` 到识别出的定义。
4. **MVU schema 合并** — 从卡 extensions 探测 MVU `stat_data` 字段，合并进每个 definition 的 `variable_schema`。
5. **角色类型解析** — 支持英文（protagonist/supporting/extra）和中文（主角/临场/龙套）。
6. **`emit_characters` 工具注册** — 让模型声明输出完成，handler 原样返回 args。

### 降级路径

- 识别失败时返回 `ExtractError::Parse`，调用方用 `CharacterDefinition::fallback_from_character(character, mvu_schema)` 把整张卡当单一主角降级。
- 不破坏该降级路径是后续增强的硬约束。

### 测试覆盖

11 个测试全部通过（`cargo test -p storyforge-app-agent -- character_extractor`）：

| 测试 | 覆盖层 |
|------|--------|
| `test_parse_layer1_tool_call` | 层 1：工具调用 |
| `test_parse_layer2_whole_json` | 层 2：整体 JSON |
| `test_parse_layer3_json_codeblock` | 层 3：JSON 代码块 |
| `test_parse_layer5_braces_among_text` | 层 5：括号配平 |
| `test_parse_role_type_chinese` | 中文角色类型 |
| `test_parse_failure_returns_error` | 解析失败兜底 |
| `test_attach_definitions_sets_card_id` | card_id 回填 |
| `test_extract_characters_with_mock_llm` | MockLlmClient 端到端闭环 |
| `test_make_config_has_unique_role` | config 角色标识 |
| `test_system_prompt_mentions_character_recognition` | prompt 关键词 |
| `test_build_user_msg_includes_card_fields` | 用户消息拼装 |

### 当前缺口

- **调用方集成验证**：已接入 `tauri-app` 导入流程；导入后会触发识别并把结果写入 CampaignStore，识别失败时保存 fallback definition。
- **识别结果 review UI**：当前无 preview/accept 流程，LLM 直接产出定义，用户无法编辑或否决。
- **重跑识别**：Campaign 面板已支持手动重跑；若卡已创建 Campaign，为避免断开既有 `definition_id` 引用，后端会拒绝重跑并提示重新导入新卡。

## 后续增强方向（未实现）

> 以下为提升识别准确率与可信度的设计方向，当前均未实现。核心动机：确定性解析无法单独解决 worldbook-only 角色的识别问题——worldbook 条目格式不统一（角色名做 key / 描述性短语 / 触发关键词 / 角色设定 / 世界规则混杂），规则引擎无法可靠区分「角色设定条目」与「世界规则条目」。

### 核心设计：两阶段流水线

1. **Stage 1 — 确定性候选提取**：用规则从 card description、personality、scenario、first_mes、alternate_greetings、mes_example 和 worldbook 条目中提取可能是角色名的候选。宁多勿漏，允许误报。
2. **Stage 2 — LLM 结构化提取**：把候选列表和原文一起给 LLM，让 LLM 判断每个候选是否是真正的独立角色，并输出结构化 `CharacterDefinition` 加 evidence（引用原文片段）和 confidence（0.0-1.0）。LLM 可否定 Stage 1 的候选，也可发现 Stage 1 遗漏的角色。
3. **Stage 3 — 用户审阅**：前端展示识别结果，每个角色显示 evidence 和 confidence，用户可接受、编辑或删除。

### 提议数据模型（临时，不持久化）

```rust
/// 确定性阶段产出的候选角色
pub struct ExtractionCandidate {
    pub name: String,
    pub source: CandidateSource,
    pub evidence_text: String,
}

pub enum CandidateSource {
    CardDescription,
    CardDialogue,
    WorldbookKey { entry_index: usize },
    WorldbookContent { entry_index: usize },
    DialogueExample,
}

/// LLM 提取产出，带置信度和证据
pub struct ExtractionResult {
    pub definition: CharacterDefinition,
    pub confidence: f32,
    pub evidence: Vec<String>,
    pub matched_candidate: Option<String>,
    pub is_independent: bool,
}

/// 完整提取报告，供前端审阅
pub struct ExtractionReport {
    pub candidates: Vec<ExtractionCandidate>,
    pub results: Vec<ExtractionResult>,
    pub rejected_candidates: Vec<ExtractionCandidate>,
    pub llm_discoveries: Vec<ExtractionResult>,
    pub metadata: ExtractionMetadata,
}
```

### 增强阶段（未执行）

- **阶段 1**：实现 `extract_candidates(character) -> Vec<ExtractionCandidate>`，从 card 各字段和 worldbook 提取候选，去重，上限 20 个。
- **阶段 2**：LLM 输出增加 `confidence` / `evidence` / `is_independent` 字段，解析产出 `Vec<ExtractionResult>`（向后兼容旧格式）。
- **阶段 3**：`build_character_extractor_user_msg` 注入候选列表，引导 LLM 逐个判断。
- **阶段 4**：组装 `ExtractionReport`，Tauri command 返回 `ExtractionReportDto`，前端适配。
- **阶段 5**：前端审阅 UI（confidence 标签 + evidence 展开 + 接受/编辑/删除 + 批量接受高置信度）。
- **阶段 6**：诊断日志（候选数量分布、LLM 置信度分布、被否候选及原因、失败时展示候选+原始输出+降级说明）。

### 增强阶段的安全约束

- 不让 `app-agent` 或 `app-pipeline` 依赖 `tauri-app`。
- 不破坏现有降级路径（`fallback_from_character` 仍保留）。
- 不改变 `CharacterDefinition` 的持久化 schema（evidence/confidence 是提取阶段临时数据，不落盘）。
- 不做通用 NER 或实体抽取管道，用 LLM 做语义判断。

### 故障模式

1. **LLM 完全不输出 JSON**：5 层兜底解析仍生效，降级到 `fallback_from_character`。
2. **LLM 输出 JSON 但缺 confidence/evidence**：向后兼容，用默认值。
3. **确定性候选提取器抛异常**：跳过候选注入，走无候选的原始流程。
4. **候选过多导致 token 超限**：截断到 20 个候选，worldbook 条目已有长度截断。
5. **用户删除所有角色**：不允许确认，至少保留 1 个角色定义。

## 验证

```bash
cargo test -p storyforge-app-agent -- character_extractor
```

当前结果（已实现部分）：11 passed, 0 failed。

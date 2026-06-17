# 计划：角色识别增强（Character Extraction）

> 状态：计划（未实现）
> 目标读者：可交给小模型按阶段执行
> 关联：`crates/app-agent/src/character_extractor.rs`、`crates/app-agent/src/prompts/character_extractor.rs`、`crates/domain/src/character.rs`、`docs/AGENT_INTERFACES.md`

## 目标

增强当前角色识别流程，使其能更可靠地从 SillyTavern 角色卡中识别所有角色，特别是隐藏在 worldbook/world info 条目中的 NPC 和配角。

完成后：

- 角色识别准确率提升，尤其是 worldbook-only 角色。
- 每个识别出的角色带有证据来源和置信度，用户可在前端审阅和修正。
- 确定性预提取作为候选筛选层，LLM 结构化提取作为语义理解层，两者结合。
- 识别失败时有清晰的降级路径和用户可见的诊断信息。

## 核心设计原则

**确定性解析无法单独解决问题。** Worldbook 条目的格式和命名约定不统一：有的用角色名做 key，有的用描述性短语，有的用触发关键词，有的条目内容是角色设定，有的是世界规则或游戏机制。规则引擎无法可靠区分"这是一个角色的设定条目"和"这是一个世界规则条目"。

因此本计划采用**两阶段流水线**：

1. **确定性候选提取（Stage 1）**：用规则从 card description、personality、scenario、first_mes、alternate_greetings、mes_example 和 worldbook 条目中提取可能是角色名的候选。这一步宁多勿漏，允许误报。
2. **LLM 结构化提取（Stage 2）**：把候选列表和原文一起给 LLM，让 LLM 判断每个候选是否是真正的独立角色，并输出结构化的 CharacterDefinition 加 evidence（引用原文片段）和 confidence（0.0-1.0）。LLM 可以否定确定性阶段的候选，也可以发现确定性阶段遗漏的角色。
3. **用户审阅（Stage 3）**：前端展示识别结果，每个角色显示 evidence 和 confidence，用户可以接受、编辑或删除。

## 非目标和安全约束

- 不让 `app-agent` 或 `app-pipeline` 依赖 `tauri-app`。
- 不破坏现有识别流程的降级路径（`fallback_from_character` 仍然存在）。
- 不改变 `CharacterDefinition` 的持久化 schema（evidence/confidence 是提取阶段的临时数据，不落盘）。
- 不做通用 NER 或实体抽取管道，用 LLM 做语义判断。
- 不在 Phase 1 改前端 UI（先改后端提取逻辑和输出格式）。

## 当前代码事实

### 角色识别 Agent（`app-agent/src/character_extractor.rs`）

- `extract_characters(runtime, character, mvu_schema, cancel)` 是主入口。
- 调用 `AgentRuntime::run_tool_loop` 跑识别 Agent，最大 8 轮工具调用。
- Agent 使用 `AgentRole::CharacterExtractor` 角色。
- 模型硬编码为 `"deepseek-chat"`（`make_character_extractor_config` 中）。
- 输入：`build_character_extractor_user_msg(character)` 构造用户消息，包含卡名、描述、性格、场景设定、系统提示词、开场白、备选开场、对话示例、世界书条目全文。
- 输出解析：`parse_character_definitions_from_response` 有 5 层兜底（工具调用 → 整体 JSON → ```json 代码块 → 裸代码块 → 括号配平）。
- 每层解析产出 `Vec<CharacterDefDto>`，再转为 `Vec<CharacterDefinition>`。
- 失败时返回 `ExtractError::Parse`，调用方（`tauri-app/lib.rs::extract_characters`）用 `fallback_from_character` 降级。

### 角色识别提示词（`app-agent/src/prompts/character_extractor.rs`）

- `CHARACTER_EXTRACTOR_SYSTEM_PROMPT`：要求 LLM 识别所有有独立人格、会说话/会行动的角色。
- 识别要点：角色名要具体，persona_prompt 用第二人称，behavior_rules 写成可执行约束，base_backstory 只写游戏开始前已知的事，宁少勿错。
- 输出格式：JSON 数组，每个元素有 name/persona_prompt/behavior_rules/base_backstory/role_type/group。
- `emit_characters` 工具：让模型声明产出，handler 原样返回 args。
- 世界书条目已作为输入传给 LLM（`build_character_extractor_user_msg` 中 `embedded_world_info` 部分）。

### CharacterDefinition（`domain/src/character.rs`）

- 字段：id、card_id、name、persona_prompt、behavior_rules、base_backstory、group、role_type、variable_schema。
- `fallback_from_character(character, mvu_schema)`：识别失败时的降级，把整张卡当单一主角。
- `RoleType`：Protagonist、Supporting、Extra。
- 没有 evidence 或 confidence 字段（这些是提取阶段临时数据，不持久化）。

### WorldInfoEntry（`domain/src/world_info.rs`）

- 字段：st_id、keys、secondary_keys、content、constant、vector_weight、extensions。
- `WorldInfoBook` 包含 `entries: Vec<WorldInfoEntry>`。
- ST 的 `character_book` 在导入时解析为 `embedded_world_info: Option<WorldInfoBook>`。

### 前端提取入口（`tauri-app/src/lib.rs`）

- `extract_characters(source_character_id)` Tauri command。
- 从 store 加载 `Character`，探测 MVU schema，调 `storyforge_app_agent::extract_characters`。
- 成功后创建 `CharacterCard` 并保存 definitions；失败时用 `fallback_from_character` 降级。
- 前端 `CampaignPanel.vue` 调用 `extractCharacters(card.source_character_id)` 触发。

## 提议数据模型

### ExtractionCandidate（临时，不持久化）

```rust
/// 确定性阶段产出的候选角色
pub struct ExtractionCandidate {
    /// 候选角色名（从 worldbook key、description 等提取）
    pub name: String,
    /// 候选来源
    pub source: CandidateSource,
    /// 原文片段（用于 LLM 判断和用户审阅）
    pub evidence_text: String,
}

pub enum CandidateSource {
    /// 从 card description / personality 中提取
    CardDescription,
    /// 从 scenario / first_mes / alternate_greetings 中提取
    CardDialogue,
    /// 从 worldbook entry key 中提取
    WorldbookKey { entry_index: usize },
    /// 从 worldbook entry content 中提取
    WorldbookContent { entry_index: usize },
    /// 从 mes_example 中提取
    DialogueExample,
}
```

### ExtractionResult（临时，不持久化）

```rust
/// LLM 提取产出，带置信度和证据
pub struct ExtractionResult {
    pub definition: CharacterDefinition,
    /// LLM 对该角色存在的置信度（0.0-1.0）
    pub confidence: f32,
    /// LLM 引用的原文证据片段
    pub evidence: Vec<String>,
    /// 确定性阶段是否已发现该候选
    pub matched_candidate: Option<String>,
    /// LLM 是否认为这是独立角色（vs 主角的一部分）
    pub is_independent: bool,
}
```

### ExtractionReport（临时，不持久化）

```rust
/// 完整提取报告，供前端审阅
pub struct ExtractionReport {
    /// 确定性阶段产出的候选
    pub candidates: Vec<ExtractionCandidate>,
    /// LLM 阶段产出的结果
    pub results: Vec<ExtractionResult>,
    /// 确定性阶段有但 LLM 否定的候选
    pub rejected_candidates: Vec<ExtractionCandidate>,
    /// LLM 新增的（确定性阶段未发现的）角色
    pub llm_discoveries: Vec<ExtractionResult>,
    /// 使用的模型和耗时
    pub metadata: ExtractionMetadata,
}

pub struct ExtractionMetadata {
    pub model: String,
    pub deterministic_candidate_count: usize,
    pub llm_result_count: usize,
    pub llm_rounds_used: u32,
}
```

## 执行阶段

### 阶段 0：审计基线

目标：确认当前识别流程的准确率边界和失败模式。

改动文件：无。

操作：

1. 运行 `cargo test -p storyforge-app-agent`，确认 character_extractor 测试全过。
2. 列出当前 `build_character_extractor_user_msg` 已包含的输入字段。
3. 列出当前 `CHARACTER_EXTRACTOR_SYSTEM_PROMPT` 的识别要点。
4. 确认 worldbook 条目已作为输入传给 LLM。
5. 记录当前 `parse_character_definitions_from_response` 的 5 层兜底逻辑。

验收：

- 有一份当前识别流程的能力和限制清单。
- 不产生代码改动。

### 阶段 1：确定性候选提取器

目标：从 card 各字段和 worldbook 条目中提取可能是角色名的候选列表。

改动文件：

- `crates/app-agent/src/character_extractor.rs`（新增 `extract_candidates` 函数）
- `crates/app-agent/src/character_extractor.rs`（新增测试）

任务：

1. 实现 `extract_candidates(character: &Character) -> Vec<ExtractionCandidate>`。
2. 候选来源：
   - 从 card description / personality 中提取人名模式（中文名、英文名、称呼词后的名字）。
   - 从 scenario / first_mes 中提取对话角色标记（如 "XXX：" 或 `*XXX*`）。
   - 从 worldbook entry keys 中提取可能是角色名的 key（排除明显的机制词如 "触发"、"规则"、"状态"）。
   - 从 worldbook entry content 中提取与 description 中出现的相同名字。
3. 去重：同名候选合并，保留所有来源。
4. 宁多勿漏：允许误报，后续由 LLM 过滤。

验证：

```bash
cargo test -p storyforge-app-agent
```

验收：

- 对包含多角色 description 的卡，能提取出多个候选。
- 对包含角色名 worldbook key 的卡，能提取出对应候选。
- 不产生误报过多导致 LLM 输入过长的情况（候选上限合理）。

### 阶段 2：LLM 提取增强（evidence + confidence）

目标：修改 LLM 提取流程，让输出包含 evidence 和 confidence。

改动文件：

- `crates/app-agent/src/prompts/character_extractor.rs`（修改 SYSTEM_PROMPT 和输出格式）
- `crates/app-agent/src/character_extractor.rs`（修改解析逻辑）
- `crates/app-agent/src/character_extractor.rs`（更新测试）

任务：

1. 修改 `CHARACTER_EXTRACTOR_SYSTEM_PROMPT`：
   - 新增输出字段 `confidence`（0.0-1.0 浮点数）。
   - 新增输出字段 `evidence`（字符串数组，引用原文片段说明为什么这是独立角色）。
   - 新增输出字段 `is_independent`（布尔值，是否是独立角色 vs 主角的一部分）。
   - 新增识别指令：当输入包含候选列表时，逐个判断是否是独立角色。
2. 修改 `emit_characters` 工具 schema，增加 confidence、evidence、is_independent 字段。
3. 修改 `CharacterDefDto` 增加可选 confidence/evidence/is_independent 字段。
4. 修改 `parse_character_definitions_from_response` 产出 `Vec<ExtractionResult>` 而非 `Vec<CharacterDefinition>`。
5. 保持向后兼容：如果 LLM 没输出 confidence/evidence，用默认值（confidence=0.8, evidence=空, is_independent=true）。
6. `extract_characters` 的返回类型改为 `Vec<ExtractionResult>`，调用方需要适配。

验证：

```bash
cargo test -p storyforge-app-agent
```

验收：

- LLM 输出包含 evidence 和 confidence。
- 旧格式输出仍能解析（向后兼容）。
- confidence 和 evidence 正确传递到 ExtractionResult。

### 阶段 3：候选注入 LLM 输入

目标：把确定性阶段的候选列表注入 LLM 用户消息，引导 LLM 逐个判断。

改动文件：

- `crates/app-agent/src/prompts/character_extractor.rs`（修改 `build_character_extractor_user_msg`）
- `crates/app-agent/src/character_extractor.rs`（修改 `extract_characters` 流程）

任务：

1. `build_character_extractor_user_msg` 增加可选参数 `candidates: &[ExtractionCandidate]`。
2. 当 candidates 非空时，在用户消息末尾追加候选列表，格式：
   ```
   【确定性预提取候选】
   以下是通过规则预提取的可能是角色名的候选，请逐个判断是否是独立角色：
   1. "林医生"（来源：card description）
   2. "陈警官"（来源：worldbook 条目 3 的 key）
   3. "神秘人"（来源：worldbook 条目 7 的 content）
   如果某个候选不是独立角色，在输出中将其 is_independent 设为 false。
   如果你发现了候选列表之外的角色，也请一并输出。
   ```
3. `extract_characters` 流程改为：
   - 先调 `extract_candidates` 获取候选。
   - 把候选注入用户消息。
   - 调 LLM。
   - 解析产出 ExtractionResult。
4. 候选数量上限：如果超过 20 个，只保留置信度最高的 20 个（按来源优先级排序）。

验证：

```bash
cargo test -p storyforge-app-agent
```

验收：

- 候选列表正确出现在 LLM 用户消息中。
- LLM 能识别候选并给出 is_independent 判断。
- LLM 能发现候选之外的角色。

### 阶段 4：ExtractionReport 组装和 Tauri 层适配

目标：组装完整提取报告，适配 Tauri command 返回类型。

改动文件：

- `crates/app-agent/src/character_extractor.rs`（新增 `ExtractionReport` 组装逻辑）
- `crates/tauri-app/src/lib.rs`（修改 `extract_characters` command 返回类型）
- `crates/tauri-app/src/lib.rs`（新增 DTO）
- `frontend/src/tauri-api.js`（适配新返回类型）

任务：

1. 新增 `ExtractionReportDto` 用于 Tauri 序列化（confidence、evidence 等字段）。
2. `extract_characters` 的 Tauri command 返回 `ExtractionReportDto` 而非 `CardSummaryDto`。
3. 保持向后兼容：前端收到新格式后，仍可取出 definitions 创建 CharacterCard。
4. 确定性阶段有但 LLM 否定的候选标记为 `rejected_candidates`。
5. LLM 新增的（确定性阶段未发现的）标记为 `llm_discoveries`。

验证：

```bash
cargo test -p storyforge
cd frontend && npm run build
```

验收：

- Tauri command 返回 ExtractionReport。
- 前端能正确解析新返回类型。
- CharacterCard 创建逻辑仍正常工作。

### 阶段 5：前端审阅 UI

目标：用户可在前端查看识别结果的 evidence 和 confidence，接受、编辑或删除。

改动文件：

- `frontend/src/components/CampaignPanel.vue`（修改识别结果展示）
- `frontend/src/tauri-api.js`（适配新返回类型）

任务：

1. 识别结果展示：
   - 每个角色卡片显示 confidence 标签（高/中/低 或颜色编码）。
   - 展开后显示 evidence（LLM 引用的原文片段）。
   - 标记 LLM 新发现的角色（确定性阶段未发现的）。
2. 用户操作：
   - 接受：保留该角色定义。
   - 编辑：修改 name/persona_prompt/behavior_rules。
   - 删除：移除该角色定义。
   - 批量接受：一键接受所有高置信度角色。
3. 确认后才创建 CharacterCard 和 definitions。

验证：

```bash
cd frontend && npm run build
```

验收：

- 用户能看到每个角色的 evidence 和 confidence。
- 用户能编辑和删除角色定义。
- 确认后才持久化。

### 阶段 6：诊断和日志

目标：提取过程可诊断，失败时有清晰的用户反馈。

改动文件：

- `crates/app-agent/src/character_extractor.rs`（增加日志）
- `crates/tauri-app/src/lib.rs`（改进错误处理）

任务：

1. 记录确定性候选数量和来源分布。
2. 记录 LLM 产出的角色数量、置信度分布。
3. 记录被否定的候选及其原因（如果 LLM 提供了）。
4. 识别失败时，向用户展示：候选列表（如有）、LLM 原始输出片段、降级说明。

验证：

```bash
cargo test --workspace
```

验收：

- 日志中能看到完整的提取过程。
- 失败时用户能看到有意义的错误信息。

## 风险和回滚策略

| 风险 | 缓解 |
|------|------|
| 确定性候选提取产生过多噪音 | 设上限（20 个），按来源优先级排序 |
| LLM 不输出 evidence/confidence | 向后兼容：缺失时用默认值 |
| 候选注入导致 LLM token 超限 | 候选列表精简（只放名字和来源），worldbook 条目已有长度限制 |
| ExtractionResult 返回类型破坏现有调用方 | 阶段 4 保持 CharacterDefinition 可从 ExtractionResult 取出 |
| 前端 UI 改动过大 | 阶段 5 先做最小展示，不做复杂编辑器 |

回滚：每个阶段独立可回滚。阶段 1 只是新增函数，不影响现有逻辑。阶段 2-3 改 LLM 提取流程但保持旧输出格式兼容。阶段 4-6 是展示层改动。

## 验证命令

```bash
# 每阶段完成后
cargo test -p storyforge-app-agent          # 阶段 1-3
cargo test -p storyforge                    # 阶段 4
cd frontend && npm run build                # 阶段 5
cargo test --workspace                      # 阶段 6
```

## 故障模式

1. **LLM 完全不输出 JSON**：5 层兜底解析仍生效，降级到 `fallback_from_character`。
2. **LLM 输出 JSON 但缺少 confidence/evidence**：向后兼容，用默认值。
3. **确定性候选提取器抛异常**：跳过候选注入，直接用无候选的原始流程。
4. **候选过多导致 token 超限**：截断到 20 个候选，worldbook 条目已有长度截断。
5. **用户删除所有角色**：不允许确认，至少保留 1 个角色定义。

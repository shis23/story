# 角色识别计划（Character Extraction）

> 状态：已实现核心能力（2026-06-15 完成）
> 关联：`docs/AGENT_INTERFACES.md` §6.2、`docs/DOCS-CODE-AUDIT.md`

## 目标

导入 ST 角色卡时，自动识别卡内包含的多个角色，为每个角色生成 `CharacterDefinition`，支撑 Campaign 多角色写作。

## 非目标

- 不做运行时动态角色识别（识别只在导入时跑一次）。
- 不做用户手动编辑识别结果的 UI（当前由 LLM 直接产出，无 preview/accept 流程）。
- 不做导出回 ST 卡格式（见 `PLAN-ST-IMPORT-EXPORT.md`）。

## 当前事实

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

- **调用方集成**：`extract_characters` 在 `tauri-app` 导入流程中的调用点未在本次审计中验证（可能已接入，需确认 `tauri-app` 侧）。
- **识别结果 review UI**：当前无 preview/accept 流程，LLM 直接产出定义，用户无法编辑或否决。
- **重跑识别**：不支持对已导入卡重新跑识别（如用户对结果不满意）。

## 验证

```bash
cargo test -p storyforge-app-agent -- character_extractor
```

当前结果：11 passed, 0 failed。

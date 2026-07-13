# Import/Export Real Fixture Corpus Result

- 分支：`codex/import-fixture-corpus`
- 基线：`b46ddc8`
- 代码收口 HEAD（最后一个代码/测试提交）：`2130fda`（`2130fda9e23d7d8596eb4c106a734099af53fa75`）
- 本文档系列最终 HEAD：见 `git rev-parse HEAD`（文档提交会移动 HEAD，故以实际为准）
- 工作目录：`C:\tmp\storyforge-import-corpus`
- 日期：2026-07-13

## 结论

把既有 ST/Campaign 兼容矩阵扩展为更大的生成式/脱敏 fixture 语料库，覆盖首次导入
→ 导出 → 重导三段对比、多固定 seed 属性测试、异常输入与原子性、以及稳定的
JSON + Markdown 报告。全程 TDD（红→绿），未调用真实/付费 LLM，未操作 GUI，未 push，
未改 `tauri-app` bundle 命令 / SQLite / `main` / `docs/HANDOFF.md` /
`docs/RELEASE-CHECKLIST.md`。

## 分支与提交

```text
2130fda fix(import): close evidence-chain P1s in fixture corpus reports
1799836 feat(import): privacy-safe real fixture evidence and JSON+Markdown reports
6b4ae6f test(import): atomic no-partial-store robustness and size/bomb guards
f4ffbae feat(import): compat matrix schema, multi-seed property suite, fail-closed completeness
3d01b43 test(import): expand sanitized fixture corpus and generators
ace1089 docs(workstream): plan import fixture corpus
```

修改文件（相对基线 `b46ddc8`）：

- `crates/infra-import/src/compat.rs` — 矩阵 schema/version、矩阵行、fail-closed 完整性、
  source→first-import 对比、3 个新生成器、5 个固定 seed、脱敏失败输出、real-card 脱敏
  证据、Markdown 报告。
- `crates/infra-import/src/lib.rs` — real-card ignored 测试改为 fail-clearly 并产出脱敏证据；
  新增 empty chara / 尺寸炸弹 / all-or-nothing 原子性红测。
- `crates/infra-import/src/png.rs` — 新增 bad-CRC / oversized-chunk / truncated-chunk /
  dangling-header 结构性红测。
- `crates/infra-import/fixtures/st_v3_large_worldbook.json` — 120 active + 5 disabled 条目。
- `crates/infra-import/fixtures/st_v3_reasoning_regex.json` — Reasoning(6)/Input(1)/Output(2)
  正则 + minDepth/maxDepth。
- `crates/infra-import/fixtures/st_v3_mvu_tavernhelper.json` — MVU/stat_data/initvar +
  tavern_helper + 多角色定义（含同名）。
- `crates/infra-import/Cargo.toml` / `Cargo.lock` — 新增 `sha2`（脱敏指纹，workspace 既有依赖）。
- `scripts/generate-import-export-compat-report.ps1` — 从真实 CompatReport 落盘 JSON+Markdown
  （rows/findings/summary），并校验非空矩阵。
- `docs/workstreams/IMPORT-FIXTURE-CORPUS-{PROMPT,PLAN,RESULT}.md` — 计划与结果。

无 `tauri-app` / `infra-sqlite` / `main.rs` / `HANDOFF.md` / `RELEASE-CHECKLIST.md` 改动。

## P1 证据链返修（2026-07-13）

| P1 | 修复 |
| --- | --- |
| 报告脚本 Markdown 空章节 | 不再把 `$Inventory` 先转 JSON 字符串再读属性；脚本只校验并复制真实 CompatReport 产物 |
| JSON 硬编码清单 | `build_corpus_compat_report` + `emit_corpus_report_for_script` 写出含 rows/findings/summary 的真实报告 |
| `assert_matrix_complete` 可被 Complete 遮蔽 Incomplete | 改为唯一 `fixture_id` + exact set 匹配；重复/额外/缺失均 fail-closed |
| multi-seed 只跑 import→export→reimport | multi-seed 与 corpus builder 均同时跑 source→first-import 与 round-trip 两腿 |
| 真实卡证据未打印/落盘 | ignored 测试 `println!` + 写 `artifacts/.../real-card-evidence.json` |
| SHA-256 被描述为完全匿名 | 字段改为 `fingerprint_privacy=stable-linkable-not-anonymous`；未知 extension key 只计数量不回显名字 |

返修后 `scripts/generate-import-export-compat-report.ps1` 实测：
`rows=11, findings≈18848, loss=0, preserved≈12937`，Markdown 含完整 Matrix rows。

## Fixture 语料库

| Fixture | 路径 | 内容 |
| --- | --- | --- |
| V2 minimal | `crates/infra-import/fixtures/st_v2_minimal.json` | 最小 V2（既有） |
| V3 matrix | `crates/infra-import/fixtures/st_v3_matrix.json` | extensions/regex/MVU/book/aliases（既有） |
| V3 BOM | `crates/infra-import/fixtures/st_v3_matrix.bom.json` | UTF-8 BOM 前缀（既有） |
| **V3 大世界书** | `crates/infra-import/fixtures/st_v3_large_worldbook.json` | 120 active（constant/selective/both）+ 5 disabled；`key`/`keysecondary` 别名；数字与字符串 position；entry extensions |
| **V3 Reasoning 正则** | `crates/infra-import/fixtures/st_v3_reasoning_regex.json` | placement 6（Reasoning）/1（Input）/2（Output）；minDepth/maxDepth；markdownOnly/promptOnly；display HTML |
| **V3 MVU/TavernHelper** | `crates/infra-import/fixtures/st_v3_mvu_tavernhelper.json` | stat_data + mvu.initvar + depth_prompt.variables + tavern_helper + 多角色定义（含同名实例） |

生成器（deterministic，seeded）：

- `generate_edge_card_json`（既有）
- `generate_large_worldbook_card`（新）
- `generate_reasoning_regex_card`（新）
- `generate_mvu_tavernhelper_card`（新）

全部脱敏/合成；无真实卡内容、无私人文本、无密钥。

## 真实卡状态

`test_real_complex_card_fixture_preserves_core_st_fields` 仍为 `#[ignore]`。
本工作树（clean checkout）**没有** 本地 `test-card.png`，也没有
`SF_COMPLEX_CARD_FIXTURE`，因此真实卡 smoke 无法运行：

```text
powershell .\scripts\run-real-card-smoke.ps1 -SkipTauriOnLoaderError
# Complex card fixture not found: ...\test-card.png
```

这是按计划的 fail-closed 行为：**real-corpus 模式下缺失真实 fixture 必须明确失败**，
而非静默跳过。runner 现在会在缺失时抛出明确诊断信息。一旦在具备 `test-card.png` 的
机器上运行，ignored 测试会额外产出脱敏证据（计数 + 扩展键集 + SHA-256 指纹），并断言
该证据不泄露卡名/正文/lore。**本工作树未执行真实卡导入**，此项如实记录为「待真实 fixture」。

## 属性矩阵

- **对比段（legs）**：
  1. source JSON → 首次 import（`compare_source_to_first_import`，独立于后续 raw JSON 拷贝）；
  2. 首次 import → export → reimport（`compare_character_roundtrip`）。
- **固定 seed**（`PROPERTY_SEEDS`）：`0xC0A75EED`、`0x5EED1234`、`0x5EED5678`、
  `0x5EED9ABC`、`0x5EEDDEF0`。
- 每个 seed × 4 生成器（edge / large_worldbook / reasoning_regex / mvu_tavernhelper）
  全部 round-trip 无 loss。
- **行分类**（5 类，满足 PLAN）：preserved / normalized-intentionally / unsupported /
  lossy-bug / not-applicable。
- **fail-closed 完整性**：`assert_matrix_complete(expected)` 在任一期望行缺失或标记为
  incomplete 时失败并点名该行。
- **可复现失败输出**：`record_failure_output(seed, case, generator, report)` 仅打印
  seed/case/分类发现（area/field_path/severity/detail），**绝不**打印 raw 卡正文、
  greeting 或 lore。

## 异常输入与原子性

| 输入 | 期望 | 红测 |
| --- | --- | --- |
| PNG 块 bad CRC | `PngError` fail-closed | `parse_png_rejects_chunk_with_bad_crc` |
| 单块 > 64 MiB | `PngError`（MAX_CHUNK_SIZE） | `parse_png_rejects_oversized_single_chunk` |
| 截断块体 | `PngError` | `parse_png_rejects_truncated_chunk_body` |
| 悬挂部分头 | 停止并返回已收集块（不 error） | `parse_png_stops_cleanly_on_dangling_partial_header` |
| 空 chara payload | `JsonError`/`NoCharacterData` | `test_import_rejects_empty_chara_payload_fail_closed` |
| > 100 MiB 导入 | `PngError`（MAX_IMPORT_SIZE，parse 前拒绝） | `test_import_rejects_oversized_total_import_before_any_parse` |
| 四种畸形（placeholder/truncated json/bad base64/bad json） | 全部 `Err`，无半成品 Character | `test_import_is_all_or_nothing_no_partial_character` |

**原子性边界**：infra-import 是纯解析（无 store），因此「no partial store」= 解析是
all-or-nothing，绝不返回半成品 `Character`。Campaign bundle 的 store 级原子性在
`tauri-app`，本分支明确不动。

## Intentional normalizations（产品边界）

- 每次导入生成新 `Character.id`。
- world book `position` 单向规范化：字符串标签（`after_char` 等）→ 数字码（导出不再还原字符串）。
- `disable: true` 条目在 import 时过滤（不进 book）。
- legacy `key`/`keysecondary` 在首次导入即规范化进 canonical `keys`/`secondary_keys`（写入 `raw_card_json`）。
- `RegexScript` 在本 crate 仅导入方向（无 export 转换）。
- Campaign/card/instance ID 在 bundle 导入时重写（既有）。

## Loss / 兼容发现清单

| 类别 | 发现 | 处置 |
| --- | --- | --- |
| preserved | 所有 typed 字段、extensions、MVU schema、regex metadata、book metadata、entry extra、alternate greetings | round-trip 稳定 |
| normalized-intentionally | `position` 字符串→数字；`key`/`keysecondary`→`keys`/`secondary_keys`；disabled 条目过滤；id 重生成 | 文档为产品边界，非 bug |
| unsupported | RegexScript export 转换（本 crate 不存在）；MVU JS→typed 规则（依赖可选 LLM，本线不调） | 文档边界 |
| lossy-bug | **本线未发现新 loss**。既有的 BOM / book metadata / key alias / 首导 raw 规范化均已在前序 hardening 线修复 | — |
| not-applicable | Turn/Attempt runtime（Bundle v2 不携带，且本线禁止扩展） | 文档边界 |

无未分类 loss；无 bad-reference 静默丢弃（opaque payload 的 broken ref 走 rejected/loss
分类，不静默吞）。

## 真实测试结果（PLAN 专项门禁）

按 PLAN 验证段执行（`$env:CARGO_TARGET_DIR=C:\tmp\storyforge-parallel-target`）：

```text
cargo fmt --all -- --check
# clean

cargo test -p storyforge-domain --lib
# 249 passed; 0 failed

cargo test -p storyforge-infra-import --lib
# 53 passed; 0 failed; 1 ignored (real fixture)

powershell .\scripts\run-real-card-smoke.ps1 -SkipTauriOnLoaderError
# FAIL: test-card.png not present in this worktree (fail-closed, as designed)
# recorded honestly; real-card import not executed here

powershell .\scripts\generate-import-export-compat-report.ps1
# wrote artifacts/import-export-compat/compat-inventory-*.json
# wrote artifacts/import-export-compat/compat-inventory-*.md
# (artifacts/ gitignored; not committed)

cargo clippy -p storyforge-domain -p storyforge-infra-import --all-targets -- -D warnings
# Finished ok

git diff --check b46ddc8..HEAD
# clean
```

测试函数总数（infra-import）：54（53 非 ignore + 1 ignored 真实卡）。本线含 P1 返修后的
证据链红→绿测试。

## 自审（数据损失 / 坏引用 / 部分写入 / 隐私）

- **数据损失**：每条发现均分类为 preserved / intentional / unsupported / lossy-bug /
  N-A；无未分类 loss。本线未发现新 loss。
- **坏引用**：opaque payload 内的 broken ref 走 rejected/loss 分类，不静默丢弃；
  源→首导对比能捕获首导阶段已发生的字段丢失。
- **部分写入**：infra-import 纯解析，all-or-nothing；四类畸形输入全部 `Err` 且无半成品
  Character。store 级原子性在 tauri-app，未动。
- **隐私**：committed fixture 全部 synthetic/sanitized（grep 真实卡名 `命定`/`seraphina`
  等均无命中）；real-card 证据仅含计数/已知扩展键 allowlist/feature flag/linkable SHA-256 指纹（非匿名）；脱敏失败输出
  不含 before/after 值；`artifacts/` 被 gitignore，不入库。唯一保留真实卡字符串处为
  ignored 测试断言（仅在本地 fixture 存在时运行，从不打印卡正文）。

## 残留边界与风险

- **真实复杂卡**：本工作树无 `test-card.png`，真实卡 smoke 未执行；需在具备本地 fixture
  的机器上跑 `scripts/run-real-card-smoke.ps1`（必要时 `-SkipTauriOnLoaderError`）补完整
  S1 证据。
- **Turn/Attempt runtime**：Bundle v2 不携带运行时 Turn 状态（本线禁止扩展）。
- **MVU JS 分析**：仅保证 deterministic schema 探测与 opaque extensions 保留；JS→typed 规则
  仍依赖可选 LLM 路径（本线不调）。
- **tauri 测试编译**依赖本地 `frontend/dist`（gitignore）；CI/本机需先有占位 dist 才能编
  `storyforge` lib tests——本线未触碰 tauri-app。
- **position / alias 规范化**：在严格字节级 ST 对比中记为 intentional（非 bug）。

## 是否建议合并

**建议合并**（独立可审、专项门禁通过、无 GUI/LLM/push 越界、无禁止区域改动）。

合并前建议 reviewer 关注：

1. `sha2` 依赖加入 infra-import（仅用于脱敏 SHA-256 指纹，workspace 既有版本，无新 vendor）。
2. real-card ignored 测试现在额外断言脱敏证据；确认该断言不破坏既有 smoke runner 行为。
3. 是否接受 intentional normalization 列表（position 单向、disabled 过滤、id 重生成、
   alias 首导规范化）为产品边界。

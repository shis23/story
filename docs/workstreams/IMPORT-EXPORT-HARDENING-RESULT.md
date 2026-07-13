# Import / Export Compatibility Hardening Result

- 分支：`codex/import-export-hardening`
- 基线：`main@c3a972d`（本线计划提交 `5b8af44`）
- 收口 HEAD：见下方 commit 列表
- 工作目录：`C:\tmp\storyforge-import-export`
- 日期：2026-07-13

## 结论

完成 ST 卡 / Campaign Bundle 导入导出兼容与损失检测的垂直切片：

1. **兼容矩阵 API**：`crates/infra-import/src/compat.rs` 提供 inventory、语义 round-trip 对比、机器可读 `CompatReport`、bounded property seed RNG。
2. **已修损失**：
   - UTF-8 BOM JSON 可导入；
   - `character_book` 顶层 metadata（name/description/scan_depth/extensions 等）经 `WorldInfoBook.metadata` 保留并写回；
   - 旧版 `key` / `keysecondary` 别名解析；
   - Campaign Bundle 中途写失败不再留下 partial card/campaign/instance（原子回滚）。
3. **语义 round-trip**：import → internal → export PNG/JSON → re-import，区分 intentional normalization 与 loss。
4. **Campaign Bundle**：扩展覆盖 variables、multi-character definitions、knowledge provenance、unsupported version fail-closed。
5. **Fixtures + report script**：仓库内 sanitized fixtures 与 `scripts/generate-import-export-compat-report.ps1`。

未调用真实/付费 LLM，未操作 GUI，未 push，未改 `docs/HANDOFF.md` / Turn lifecycle / SQLite / plugin host / release pipeline。

## Format matrix

| 格式 | 路径 | 状态 | 备注 |
| --- | --- | --- | --- |
| ST V2 JSON | `import_character_from_json` | 保真 | 缺省 `spec_version=2.0`；可选字段 default |
| ST V3 JSON | 同上 | 保真 | extensions / extra / character_book / regex / MVU schema |
| ST PNG tEXt `chara` | `import_character_from_png` / `write_st_card_png` | 保真 | CRC/截断/坏 base64/坏 JSON fail-closed |
| UTF-8 BOM JSON | `strip_utf8_bom` | 已修 | BOM 前缀不再 JsonError |
| ST key aliases | `StWorldInfoEntry.resolved_*` | 已修 | `key` / `keysecondary` |
| World book metadata | `WorldInfoBook.metadata` | 已修 | book-level extra round-trip |
| Scoped regex | `Character.scoped_regex_scripts` | 保真 | placement / promptOnly / markdownOnly 等 |
| MVU/stat_data schema | `extract_mvu_schema_from_extensions` | 探测保真 | 纯 schema；JS 分析仍走可选 LLM（本线不调用） |
| Preset regex metadata | `import_preset` | 保真 | 既有测试维持 |
| Campaign JSON Bundle v2 | export/import | 加强 | 原子导入 + 变量/多角色/知识溯源 |
| Campaign → multi ST PNG + lorebook | `export_campaign_st_cards` | 有意降级 | 每实例一张卡 + 共享 lorebook，不 flatten 成单卡 |
| Turn/Attempt/Chronicle runtime | Bundle | 产品缺口 | Bundle v2 不含 Turn/Attempt 运行时记录；Chronicle A 仅 summaries |

## Fixed losses

| 问题 | 红测证据 | 修复 |
| --- | --- | --- |
| BOM JSON 导入失败 | `compat::tests::utf8_bom_prefixed_json_imports_successfully` 曾 `JsonError` | `strip_utf8_bom` |
| book-level metadata 丢失 | `world_book_metadata_survives_import_export_reimport` 曾 `metadata.name=None` | `from_st`/`to_st_book` 保留 `extra` |
| `key`/`keysecondary` 丢主键 | `st_key_aliases_key_and_keysecondary_import` 曾 keys=`[]` | alias 字段 + resolved helpers |
| Bundle 中途失败 partial write | `import_campaign_bundle_is_atomic_on_mid_write_failure` 曾留下 card | 先重写再写；失败 `delete_campaign`/`delete_card`/`conv.delete` 回滚 |

## Intentional degradations / normalizations

- 每次导入生成新 `Character.id` / Bundle 导入重写全部 ID 与引用。
- world book `position` 可能从 ST 字符串标签规范为数字码。
- 禁用 world book entries 在 import 时过滤。
- 多角色 Campaign 导出 ST 时 **一张角色一张 PNG + 共享 lorebook**，不静默 flatten 成单卡。
- 变量/任务/知识等 StoryForge 语义不保证映射回完整 ST 卡语义（ST 无对等概念处保持 Bundle 保真）。
- MVU 复杂 JS → typed 规则仍依赖可选 LLM 分析路径；本线只保证 deterministic schema 探测与 opaque extensions 保留。

## Fixture inventory

| Fixture | 路径 | 内容 |
| --- | --- | --- |
| V2 minimal | `crates/infra-import/fixtures/st_v2_minimal.json` | 最小 V2 |
| V3 matrix | `crates/infra-import/fixtures/st_v3_matrix.json` | extensions/regex/MVU/book/aliases |
| V3 BOM | `crates/infra-import/fixtures/st_v3_matrix.bom.json` | UTF-8 BOM 前缀 |
| Generated edge | `compat::generate_edge_card_json` | bounded property，seed `0xC0A75EED`，24 cases |

无外部用户卡；无密钥。

## 红测 → 绿测证据

### ST / infra-import

1. **RED**（实现前）：
   - BOM → `JsonError(expected value line 1)`
   - book metadata → `None`
   - key aliases → `keys=[]`
2. **GREEN**（当前）：
   ```text
   cargo test -p storyforge-infra-import --lib
   # 25 passed; 0 failed; 1 ignored
   ```
   含：
   - `utf8_bom_prefixed_json_imports_successfully`
   - `world_book_metadata_survives_import_export_reimport`
   - `st_key_aliases_key_and_keysecondary_import`
   - `semantic_roundtrip_report_has_no_losses_for_matrix_fixture`
   - `bounded_property_suite_roundtrips_without_loss`
   - `inventory_and_report_are_machine_readable_json`
   - 既有 PNG/JSON/preset 保真测试

### Campaign bundle

1. **RED**：`import_campaign_bundle_is_atomic_on_mid_write_failure` 在实例持久化失败后仍留下 partial card。
2. **GREEN**：
   ```text
   cargo test -p storyforge --lib campaign_bundle
   # 6 passed
   ```
   覆盖 atomic、variables/multi-character/provenance、unsupported version、既有 rewrite/export 测试。

## 实际测试结果（专项门禁）

已跑（按 PLAN，非整 workspace）：

```text
cargo test -p storyforge-domain --lib
# 243 passed

cargo test -p storyforge-infra-import --lib
# 25 passed; 1 ignored (real fixture)

cargo test -p storyforge --lib campaign_bundle
# 6 passed

cargo clippy -p storyforge-domain -p storyforge-infra-import -p storyforge --all-targets -- -D warnings
# Finished ok

cargo fmt --all -- --check
# clean

powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\generate-import-export-compat-report.ps1
# wrote artifacts/import-export-compat/compat-inventory-*.json

git diff --check c3a972d..HEAD
# clean
```

## Commits（相对基线后本线）

1. `5b8af44` docs(workstream): plan import export hardening
2. `f96f3e2` fix(domain): preserve world book metadata and ST key aliases
3. `0d79ed0` test(import): add ST compatibility matrix, BOM handling, and fixtures
4. `9f3a099` fix(import): make campaign bundle import atomic and expand roundtrips
5. （本 RESULT 文档提交，若随后产生）

## 修改文件（主要）

- `crates/domain/src/character.rs` — ST key aliases + resolved helpers
- `crates/domain/src/world_info.rs` — `WorldInfoBook.metadata` round-trip
- `crates/infra-import/src/lib.rs` — BOM strip
- `crates/infra-import/src/compat.rs` — matrix / report / property suite
- `crates/infra-import/fixtures/*` — sanitized fixtures
- `crates/infra-import/src/png.rs` / app-\* call sites — `metadata` 字段编译适配
- `crates/tauri-app/src/lib.rs` — atomic bundle import + 扩展测试
- `scripts/generate-import-export-compat-report.ps1` — 报告入口
- `docs/workstreams/IMPORT-EXPORT-HARDENING-RESULT.md` — 本结果

## 未完成项与风险

- **UI-only**：前端导入对话框、真实用户卡矩阵、导出文件保存交互仍需手工验证。
- **真实复杂卡**：`test_real_complex_card_fixture_*` 仍为 `ignore`，依赖本地 `test-card.png` / `SF_COMPLEX_CARD_FIXTURE`。
- **Turn/Attempt**：Campaign Bundle 不携带运行时 Turn 状态；跨设备恢复玩法进度仍靠 Bundle 内 summaries/tasks/knowledge/variables。
- **原子性粒度**：JSON 文件 store 的回滚是 best-effort 补偿删除，不是单文件事务；并发同 store 写入仍需上层串行化。
- **position 规范化**：字符串↔数字可能在严格字节级 ST 对比中显示为 intentional diff。
- **tauri 测试编译**依赖本地存在 `frontend/dist`（gitignore）；CI/本机需先有占位 dist 才能编 `storyforge` lib tests。

## 是否建议合并

**建议合并**（独立可审、门禁通过、无 GUI/LLM/push 越界）。

合并前建议 reviewer 关注：

1. `WorldInfoBook.metadata` 序列化兼容旧存档（`#[serde(default)]`）；
2. Bundle 失败回滚是否覆盖目标部署的 store 布局；
3. 是否接受 intentional normalization 列表为产品边界。

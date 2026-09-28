# R8：两条「无记录」发现收口（M-30 文档漂移 + G-07 tag↔版本校验）

- **任务**：task-31（owner `review-goals`），第二遍审查收口
- **日期**：2026-09-13
- **上游依据**：`round2/R6-fix-completeness-audit.md` §2「无记录」清单（R6 判定 M-30、G-07 两条无人处置）
- **基线**：工作区 HEAD `ab894c6` + 本轮未提交修复工作树
- **写作用域遵守**：只改 `docs/`（本报告 + `09-goals-scripts-fixes.md` + 三处被点名文档）、`scripts/release-build/**`、`scripts/tests/**`、`.github/workflows/release.yml`。**未改** Rust/前端代码、`.gitignore`、其他域 fixes 记录、`round2/R1..R7*.md`。

## 0 结论摘要

| 发现 | 原状态（R6） | 本次最终状态 | 说明 |
|---|---|---|---|
| **M-30**（P3 文档漂移，3 子项） | 无记录 | **已修复（文档）** | ① 核对确认（代码事实）并标注"已修复(2026-09-13, task-16)"；② `ROADMAP.md:123` + `PLAN-ST-IMPORT-EXPORT.md:62/63/111` 按代码事实回写 v2/v3 语义（**代码自洽，无代码缺陷，无需 Lead 决策**）；③ `RELEASE-STATUS.md:29` 补边界限定，原「已闭合」声明**未删** |
| **G-07**（release.yml 缺 tag↔版本校验） | 无记录（09 记"转交未修"） | **已修复（脚本 + 测试 + 工作流步骤；CI 侧未运行时验证）** | 新增 `scripts/release-build/verify-tag-version.ps1` + `scripts/tests/ReleaseBuild.TagVersion.Tests.ps1`（10 个用例，含必需四类）；`.github/workflows/release.yml` windows job 加一步（构建前、tag-only 门控）；**契约测试未破，无需改契约** |

**门禁（真实数字）**：`powershell -NoProfile -ExecutionPolicy Bypass -File scripts/tests/run-release-build-tests.ps1` → **exit 0**，**Passed: 208 / Failed: 0**（51 + 14 + 34 + 99 + **10 新增**；原 198）。release.yml 用契约测试同款引擎校验：`Test-ReleaseWorkflowSyntax` → pyyaml，`Valid=True`、`ErrorCount=0`。

---

## 1 M-30 逐子项

### 1.1 M-30-① `CLAUDE.md` 的 `card_shell_clear_cache`「UI 入口未接」

**代码事实（2026-09-13 复核，非引用记录）**

| 事实 | 位置 | 证据 |
|---|---|---|
| 命令 | `card_shell_clear_cache` | Rust 命令注册表（`backend-baseline.mjs` 定义/注册 175/175） |
| 前端 wrapper | `frontend/src/tauri-api.js:1075` | `export async function cardShellClearCache()` |
| **UI 入口（已接线）** | `frontend/src/components-v2/shell/InspectorDrawer.vue:18` 导入、`:45` 调用 | `import { cardShellClearCache } from '../../tauri-api.js'` / `const n = await cardShellClearCache()` |
| 文档 | `CLAUDE.md:122` | 已写 **"UI 入口已接线（2026-09-13 更正）"** |

**判定**：原声明"UI 入口未接"**已不成立**；`CLAUDE.md` 已由 **task-16 回写**（本轮复核代码后确认回写正确，未再改动 CLAUDE.md）。按 Lead 要求标注：**已修复（2026-09-13, task-16）**。

### 1.2 M-30-② bundle `format_version` 口径

**代码事实（全链路复核，行号为当前工作树）**

| 环节 | 事实 | 位置 |
|---|---|---|
| JSON 导出 | `runtime: None` + `format_version: BUNDLE_FORMAT_VERSION`（= **2**） | `crates/tauri-app/src/commands/import_export.rs:601-603`；常量 `:22` |
| SQLite/后端导出 | 挂上 `runtime`（conversation/character/turns/world_info）后写 **3**，并立即 `validate_runtime` 自检 | `storage_backend.rs:1554` / `:1562` / `:1563` |
| SQLite runtime 导出 | `format_version: if runtime.is_some() { 3 } else { 2 }` | `sqlite_runtime.rs:278` |
| 导入（SQLite 分支） | 拒绝 `0` 与 `>3`（即接受 **1..=3**） | `import_export.rs:654` |
| 导入（JSON 分支） | 同上 | `import_export.rs:1047` |
| v3 语义 | 无 `runtime` → `validation("v3 Bundle 缺少正文快照")`；v1/v2 无 `runtime` 合法 | `commands/bundle_runtime.rs:20-23` |
| 测试锚点 | SQLite 导出断言 `format_version == 3`；JSON 夹具为 2 | `crates/tauri-app/tests/sqlite_character_lifecycle.rs:397` / `:576` |

**判定：代码自洽**——`BUNDLE_FORMAT_VERSION = 2` 只描述"无正文快照的 JSON 导出"这条路径；携正文快照的路径写 3，且导入侧对 v3 强制要求 `runtime`。"常量=2 与存在 v3 语义"不是缺陷，是**两个导出路径的版本语义**。**无需 Lead 决策**（未发现需要改代码的问题）。

**文档回写（原文本 → 新文本）**

| 文件:行 | 原文本 | 新文本（要点） |
|---|---|---|
| `docs/ROADMAP.md:123` | `- ~~设计 StoryForge Campaign 导出格式。~~ ✅ JSON bundle（\`format_version\`）` | 追加"2026-09-13 更正（task-31，M-30-②）"：导出写 v2（无正文，`import_export.rs:22/:603`）或 v3（含正文，`sqlite_runtime.rs:278`）；导入接受 1..=3（`:654/:1047`）且 v3 必须带 `runtime`（`bundle_runtime.rs:20`）；口径与代码一致，无代码缺陷 |
| `docs/PLAN-ST-IMPORT-EXPORT.md:62` | `…StoryForge 专有 JSON Bundle v2，包含完整 CharacterCard…` | 改为"JSON Bundle v2（无正文快照）…"并追加 v2/v3 双路径说明（含行号） |
| `docs/PLAN-ST-IMPORT-EXPORT.md:63` | `…v2 使用 bundle 内的完整 CharacterCard，兼容 v1 只有 definitions 的旧 bundle…` | 改为"接受 `format_version` 1..=3…v3 必须带 `runtime` 正文快照（缺失即 validation）…" |
| `docs/PLAN-ST-IMPORT-EXPORT.md:111` | `- [x] 版本号和向前兼容策略：\`format_version = 2\`；导入兼容 v1（无完整 card，仅 definitions）。` | 改为导出 2/3 双路径 + 导入 1..=3 的三档语义 + `0`/`>3` fail-closed |

### 1.3 M-30-③ `RELEASE-STATUS.md:29`「第三方插件两条通道闭合」的边界

原文本（节选）：`…第三方插件两条通道：1B 真实 TavernHelper 卡 5/5 远程脚本（含 MVU bundle）执行成功，1A manifest 插件 4 个真实缺陷修复并复验。边界与证据见下节 |`

新文本（**追加**，原声明保留）：`…边界与证据见下节。**边界限定（2026-09-13，task-31／M-30-③）**：「通道闭合」指已验证的挂载／执行／权限门控路径在验收样本上闭合，**不等于不存在可绕过路径**——子帧／权限边界的运行时验证见 P0-2（\`M-01\`：Windows 子帧 iframe 仍持有 Tauri IPC，本轮暂缓未修）；本行结论不构成插件沙箱攻击面评估（另见本文件 \`:60\` 的免责声明）。原「已闭合」判定不改写 |`

**依据**：`M-01`（P0-2）本轮状态为"暂缓（主体未修）"（`06-meta-plugin-fixes.md` 结论 1；`R6-fix-completeness-audit.md` §7）；`:60` 免责声明为"不等于密码学审计或插件沙箱攻击面评估"。**未删除、未改写**原「已闭合」声明。

---

## 2 G-07 交付物

### 2.1 新增离线校验脚本 `scripts/release-build/verify-tag-version.ps1`

| 项 | 内容 |
|---|---|
| 输入 | `-Tag <tag>`（必需）、`-RepoRoot <path>`（可省，默认 `git rev-parse --show-toplevel`）、`-PassThru` |
| 版本来源 | `crates/tauri-app/tauri.conf.json` 的 `"version"` **与** 根 `Cargo.toml` `[workspace.package] version`（`crates/tauri-app/Cargo.toml` 是 `version.workspace = true`，继承前者） |
| 规则①（前缀） | tag 可带 `v/V` 前缀，比较前剥离；无前缀同样接受 |
| 规则②（格式） | tag 必须为 `MAJOR.MINOR.PATCH` + 可选 `-prerelease` |
| 规则③（核心版本） | tag 数字核心必须等于 manifest 版本数字核心 |
| 规则④（预发布） | manifest 无 prerelease 时，tag 允许带 prerelease（`v0.1.2-rc.1` ↔ `0.1.2`）；manifest 自带 prerelease 时 tag 必须写同一整版 |
| 规则⑤（一致性） | 两个版本来源必须互相一致，否则报 drift 并失败（fail-closed） |
| 退出码 | `0` 匹配；`1` 不匹配/漂移/不可解析/文件缺失；`2` 用法错误（无 `-Tag`） |
| 兼容性 | ASCII-only、CRLF、无 BOM；PowerShell 5.1 与 7 均可（本地以 5.1 验证） |

冒烟（真实仓库）：`-Tag v0.1.2` → exit **0**；`-Tag v0.1.3` → exit **1**（`FAIL tag version 0.1.3 does not match manifest version 0.1.2`）；无 `-Tag` → exit **2**（usage）。

### 2.2 新增 Pester 套件 `scripts/tests/ReleaseBuild.TagVersion.Tests.ps1`（10 个用例，全绿）

必需四类：**匹配**（`v0.1.2`，含两条 source 行断言）、**无 v 前缀**（`0.1.2`）、**预发布后缀**（`v0.1.2-rc.1`）、**不匹配**（`v0.1.3` → exit 1）。
补充六类：manifest 两源漂移（conf 0.1.2 / workspace 0.1.3）、tag 非法（`v1.2`）、manifest 自带 prerelease 时 tag 必须重复、版本来源文件缺失 fail-closed、缺 `-Tag` → exit 2、真实仓库当前版本自洽（tag = `v` + `tauri.conf.json` 实测版本 → exit 0）。
已接入 `scripts/tests/run-release-build-tests.ps1` 的 `$testFiles`（第 5 个文件），因此进入 11 步门禁的 Pester 步骤。

### 2.3 `.github/workflows/release.yml`（windows job，构建前）

```yaml
      # G-07 (review-2026-09-13): fail closed before building when the pushed tag
      # disagrees with the application version in tauri.conf.json / Cargo.toml.
      # workflow_dispatch runs are branch-based, so this check is tag-only.
      - name: Verify tag matches app version (fail closed)
        if: startsWith(github.ref, 'refs/tags/')
        shell: pwsh
        run: pwsh -NoProfile -File scripts/release-build/verify-tag-version.ps1 -Tag '${{ github.ref_name }}'
```

- 位置：windows job 的 `Install pinned tauri-cli` 之后、`Build Windows bundle (msi + nsis)`（`cargo tauri build --ci`）**之前**（当前文件 `:80-86`）。
- 门控理由：`workflow_dispatch` 触发时 `github.ref_name` 是**分支名**而非 tag，无门控会误失败；tag push 时必定执行。
- 不发布保证：`release` job `needs: [windows, android]`（`:228`）且 `if: startsWith(github.ref, 'refs/tags/')`（`:230`）——windows job 因版本不匹配失败 ⇒ 发布步骤不会执行。
- 契约影响：**未破**。加步后 `ReleaseBuild.CI.Tests.ps1` 的 99 条静态契约（含 release.yml 的 job 头/`cargo tauri build --ci`/`SHA256SUMS-*` 命名断言）**全部通过**，因此**不需要**修改任何契约测试，也就没有"最小契约更新"这一步。YAML 经 `Test-ReleaseWorkflowSyntax`（pyyaml）验证 `Valid=True`。
- **CI 侧改动未运行时验证**：本地只能做静态契约 + YAML 解析 + 脚本离线用例；真实 tag push 的执行证据需下一次发布。

---

## 3 门禁与校验记录（真实数字）

| 校验 | 命令 | 结果 |
|---|---|---|
| 发布构建测试全量 | `powershell -NoProfile -ExecutionPolicy Bypass -File scripts/tests/run-release-build-tests.ps1` | **exit 0**；每套 `Passed: 51 / 14 / 34 / 99 / 10`，**合计 208，Failed 0，Skipped 0**（原 198 → 208，新增 10 条全绿） |
| workflow YAML | `Test-ReleaseWorkflowSyntax -Path .github/workflows/release.yml`（pyyaml） | `Valid=True`、`ErrorCount=0` |
| 脚本冒烟 | `verify-tag-version.ps1 -Tag v0.1.2 / v0.1.3 / (无)` | exit `0 / 1 / 2` |
| 编码约定 | 两个新 `.ps1` 文件 | ASCII-only（非 ASCII=0）、CRLF、无 BOM（与 `scripts/**` 既有约定一致） |
| 硬编码计数检查 | `Select-String '\b198\b' scripts/**, .github/**` | 无任何脚本硬编码 198，新增套件不会撞断言 |

---

## 4 与 R6 对账表的更新

R6 §2「无记录」4 条 → 本次收口后：

| ID | R6 状态 | R8 后状态 | 归属 |
|---|---|---|---|
| M-30 | 无记录 | **已修复（文档）** | 本任务（task-31） |
| G-07 | 无记录 | **已修复（脚本+测试+步骤；CI 未运行时验证）** | 本任务（task-31） |
| W-18 | 无记录 | 待 task-32（review-pipeline） | 他人 |
| W-31 | 无记录 | 待 task-33（review-frontend） | 他人 |

> 说明：task-31 要求"追加到 `09-goals-scripts-fixes.md` 新增 §7"，但该文件 `§7` 已被「转交清单」占用（§1–§11 已存在），故新增内容落为 **§12**，内容与要求一致（逐条"原文本→新文本→依据" / "改了什么→如何验证"）。

---

## 5 遗留与风险（诚实声明）

1. **G-07 步骤只在 windows job**：android job 未加同一校验。若 tag 与版本不匹配，windows job 会失败 ⇒ `release` job 不执行 ⇒ 不会发布；但 android job 仍会白跑一次构建。补 android 会增加契约面，本轮按"最小改动"未加（可作为后续一行改动）。
2. **G-07 边界**：校验只覆盖 `v*` tag push；`workflow_dispatch` 分支构建按设计不校验。tag 与 manifest 都一致但**语义错发**（例如用 v0.1.2 重发旧提交）不在本脚本职责内。
3. **CI 未运行时验证**：见 §2.3。
4. **未纳入本任务的相邻项**（R6/09 §11.7 已记录、本轮未动）：`docs/RELEASE-CHECKLIST.md:72/:136`、`PLAN-ST-IMPORT-EXPORT.md:127`、`DOCS-CODE-AUDIT.md:300` 与 `scripts/run-real-card-smoke.ps1` 默认夹具路径（`data/local/test-card.png`）的口径不一致——不在 task-31 派单范围，建议并入文档收尾。
5. 本报告的所有数字均为**未提交工作树**状态。

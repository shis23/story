# 修复记录 09：scripts / .gitea / artifacts 层（task-17）

- 任务：**task-17**「修复域7 剩余项：scripts / .gitea / artifacts 层」（承接 07 号报告中 G-01..G-06/G-11 之外的剩余条目）
- 日期：2026-09-13（Asia/Shanghai）；复核基线 `ab894c6`（复核期工作树含其他域的并发修复，见 §9）
- 写作用域（实际改动均在其中）：`scripts/**`、`.gitea/**`、本文件
- **明确未动**：`.github/**`（域4 正在改 `release.yml`）、Rust/前端代码、`docs/**`（唯一例外为 `docs/RELEASE-STATUS.md` 两处口径修正，理由见 §5.3）、07 号报告（评审产物，只读）
- 未跑 cargo/npm/11 步门禁（本域约束）；按 task-17 要求复跑 Pester，见 §3

---

## 1. 结论摘要

| 条目 | 处置 | 状态 | 位置 |
| --- | --- | --- | --- |
| G-06（脚本侧） | 新增停用标记；Pester 合同改为「历史合同」+ 断言 release 流程不依赖 `.gitea`、release.yml 仍为发布路径且校验和按平台分名 | **已修** | `.gitea/DECOMMISSIONED.md`（新增）；`scripts/tests/ReleaseBuild.CI.Tests.ps1`；`scripts/tests/ReleaseBuild.RunnerReadiness.Tests.ps1`；`scripts/release-build/ReleaseBuild.Common.ps1` |
| 附带发现（P1 级门禁阻断，非 07 号报告条目） | untracked 秘密扫描对**非配置文件**误用「无引号赋值」规则，导致 untracked 评审文档使门禁第 2 步失败 → 已修并加断言 | **已修** | `ReleaseBuild.Common.ps1:482,~700-760,~830`；`ReleaseBuild.CI.Tests.ps1`（untracked 用例） |
| G-12 | 方向**确认写反**（留存的是 Windows 那份）；加防回退断言；两处文档口径同步 | **已定案并修** | §5；`docs/RELEASE-STATUS.md:28,90` |
| G-07 | tag↔版本一致性校验缺失：属 `.github/**`（不在本域作用域）→ 给出 6 行补丁草案与责任边界 | 转交 | §7.1 |
| G-08 / G-09 / G-10 / G-15 / G-16 / G-18 / G-19 / G-20 | 纯文档（含 2 个被 ignore 文件）→ 给出**可直接执行**的原文本→建议文本 | 转交 task-16 | §7.2 |
| G-13 | 已在 task-14 修复（证据目录补全） | 已修（task-14） | `docs/RELEASE-STATUS.md:91` |
| G-14 | ROADMAP Phase 7 无状态：判定所需替代证据已核（§7.3）→ 属文档，转交 task-16 | 转交 | §7.3 |
| G-17 | 口径说明，非缺陷：已在本记录 §3.4 与 07 号报告 §5 固化 | 无需修改 | — |
| 不可本地验证项（Gitea 服务端 / APK 签名 / 下载实测） | 给出取证步骤与责任边界 | 转交 | §8 |

---

## 2. G-06 脚本侧：消除「工作流合同认证已停用 CI」的误导性绿灯

### 2.1 新增停用标记（`.gitea/**`，本域作用域）

新增 `/.gitea/DECOMMISSIONED.md`（2,720 B，纯 ASCII、纯 LF），内容：停用生效日 2026-09-06、用户决议、三份工作流的「历史用途 / 现状」表、停用内容（act_runner 与任务容器停止、排队 run 34-36 不再执行、`has_actions` 关闭、根因 runner 性能与工作区缓存 non-fast-forward）、现在的发布路径（`.github/workflows/release.yml` 三 job + run `34013739416`）、远端 job 只做构建（含 2026-09-13 新增的一步契约测试）而 11 步门禁只在本地、以及「看到本套 Pester 变绿不得表述为 CI 门禁通过」。

> 位置选择依据：初版放在 `.gitea/workflows/DECOMMISSIONED.md`，被同目录 legacy 断言（遍历该目录所有文件并要求每个文件都含 `timeout-minutes` / `concurrency` / `contents: read`）判为「工作流」而失败。放在 `.gitea/` 下既紧邻工作流、又不被 `*.yml` 合同误读；同时把那三条 legacy 断言**显式限定为 `*.yml`/`*.yaml`**（此前 `Get-ChildItem -File` 会把任何非工作流文件当工作流，是潜在脆弱点）。

### 2.2 Pester 标注与断言

| 文件 | 位置 | 改动 |
| --- | --- | --- |
| `scripts/tests/ReleaseBuild.CI.Tests.ps1` | 原 `:464` Describe | 更名 `ReleaseBuild legacy Gitea workflow governance checks (Gitea Actions decommissioned 2026-09-06)` + 顶部注释说明「历史合同、不得读作活跃 CI」 |
| 同上 | 该 Describe 首个 `It` | 更名并扩充为综合边界断言（见 2.3） |
| 同上 | 原 `:695` Describe | 更名 `ReleaseBuild legacy host evidence workflow defaults (Gitea Actions decommissioned 2026-09-06)` |
| 同上 | 3 条 legacy workflow 断言 | 枚举限定 `*.yml`/`*.yaml`，并注明原因 |
| `scripts/tests/ReleaseBuild.RunnerReadiness.Tests.ps1` | 原 `:1326` Describe（约 1,900 行、47 处 `.gitea` 引用） | 更名 `Release workflow static governance (LEGACY: Gitea Actions decommissioned 2026-09-06; retained as historical contract)` + 顶部 LEGACY 注释块；**内部断言一条未改** |
| `scripts/release-build/ReleaseBuild.Common.ps1` | `Assert-ReleaseWorkflowStaticContract`（原 `:6698`） | docstring 增 `.DESCRIPTION`：LEGACY、非活跃门禁、发布路径为 release.yml、入口脚本不得依赖 `.gitea`、指向本记录 |

### 2.3 新增的边界断言（在既有 `It` 内，未新增用例）

1. 标记文件存在，且含 `2026-09-06`、`decommission`、`GitHub Actions`；**显式用 UTF-8 读取**（Windows PowerShell 5.1 的 `Get-Content -Raw` 按 ANSI 解码会把多字节字符与相邻 ASCII 数字一起吞掉——本轮实测踩过，见 §3.2）。
2. 发布入口脚本 `scripts/verify-release.ps1`、`scripts/run-release-build.ps1`、`scripts/verify-release-evidence.ps1`、`scripts/tests/run-release-build-tests.ps1` 均存在且**不含 `.gitea` 引用**（防回退到停用配置）。
3. `.github/workflows/release.yml` 存在，含 `cargo tauri build --ci` 与三个 job 顶层键 `windows:` / `android:` / `release:`。
4. 校验和文件按平台分名：`SHA256SUMS*` 唯一名 ≥ 2 且**不得再出现裸名 `SHA256SUMS.txt`**（G-12 防回退）。
5. 被保留的历史工作流仍须能以真实 YAML 引擎解析（无引擎时按原逻辑 fail closed）。

### 2.4 覆盖面变化

四套 Pester 的 `It` 计数逐文件核对：`51 / 14 / 34 / 99 = 198`，与门禁记录一致——**本域修复没有增删用例，`RELEASE-STATUS.md` / `README.md` 中的「198 项」仍然成立**（命令与输出见 §9）。

---

## 3. 复跑 Pester 记录（4 次运行，含 2 次失败与根因）

命令（仓库既有入口，Pester 3.4.0 / Windows PowerShell 5.1）：
`powershell -NoProfile -ExecutionPolicy Bypass -File scripts/tests/run-release-build-tests.ps1`

| # | 结果 | 失败项 | 根因 / 处置 |
| --- | --- | --- | --- |
| A | `Tests.ps1` 50/1 失败后 fail-fast 停止 | `ReleaseBuild script encoding safety` → `keeps release PowerShell sources ASCII-only`：offender `scripts/tests/ReleaseBuild.RunnerReadiness.Tests.ps1` | 我加的注释里用了 em dash（U+2014）。该守卫要求 `scripts/**/*.ps1` 全 ASCII（PS 5.1 按 ANSI 读取会吞换行）。**换成 ASCII 分号后通过** |
| B | `Tests.ps1` 51/0、`Pipeline.Tests.ps1` 13/**1** 失败后停止 | `default repository secret scan passes without an allowlist` | 见 §4：untracked 评审文档触发「无引号赋值」假阳性；**这是本轮最重要的附带发现**，修复后该项通过 |
| C | `Tests.ps1` 51/0、`Pipeline` 14/0、`CI.Tests.ps1` 31/**3** 失败后停止 | `marks the decommissioned provider…`（`2026-09-06` 匹配失败）、`workflows use timeouts-minutes…`、`workflows use concurrency…` | ① 标记文件含中文，被 PS 5.1 `Get-Content -Raw` 按 ANSI 解码后「2026-09-06」被前一个多字节字符吞掉 → 改为 ASCII 正文 + 显式 UTF-8 读取；② 标记文件位于 `.gitea/workflows/` 被两条 legacy 断言当作工作流 → 移至 `.gitea/` 并把断言限定为 YAML 文件（见 §2.1） |
| D | **全绿** | — | `Passed: 51 / 14 / 34 / 99`，`Failed: 0`、`Skipped: 0`、`Pending: 0`、`Inconclusive: 0`，末行 `Release build tests passed.`，进程退出码 0 |

- 运行 D 的完整日志（本机临时目录，未入库）：`%TEMP%\sf-pester-task17c.log`；**最终一次全绿运行的日志副本已落 `artifacts/review-2026-09-13/pester-task17-review-goals.log`（19,367 B，说明见同目录 `task-17-pester-evidence.md`）**——`artifacts/**` 被 `.gitignore:67` 忽略，属本机证据
- 单文件耗时：9.2s / 8.5s / 16.2s / 101.9s（第 4 套约 1,900 行、含大量 AST/进程级用例）
- 结论：**Pester 四套 198 项全绿、无失败无跳过，与门禁基线一致**；`Assert-ReleasePesterResult` 的 fail-closed 语义（zero/failed/skipped/pending/inconclusive 任一即抛）本轮全程生效——三次失败都被正确拦下。

### 3.4 口径说明

- 门禁数字以**门禁日志**为准，不用静态计数核对（静态 `#[test]` 属性 2009、`#[ignore]` 52，与日志 `1980 passed + 33 ignored` 不可直接比较；见 07 号报告 G-17）。
- 本轮 Pester 复跑**只覆盖第 2 步**，不代表 11 步门禁整体；其余 10 步由 Lead 统一执行。

---

## 4. 附带修复（P1 级门禁阻断）：untracked 秘密扫描对非配置文件的假阳性

### 4.1 现象（可复现）

运行 B 中 `ReleaseBuild.Pipeline.Tests.ps1` 的 `default repository secret scan passes without an allowlist`（`{ Invoke-ReleaseSecretScan -RepoRoot $RepoRoot } | Should Not Throw`）失败，输出：

```
Potential secret material found:
  untracked unquoted secret assignment at docs/review-2026-09-13/01-domain-infra.md
```

即：**只要工作树里存在 untracked 的评审文档，11 步门禁的第 2 步（Pester）就会失败**。这不是评审文档"藏了密钥"，而是扫描器规则作用域问题——它会随本轮所有域的评审产物出现而反复触发，属门禁阻断级问题（P1）。

### 4.2 定位（不打印密钥值，符合扫描器设计）

用同一规则集对目标文件逐行匹配（只输出行号与掩码）：

- 命中位置：`docs/review-2026-09-13/01-domain-infra.md:532`（review-domain 的报告）
- 命中内容：一处 **Rust 字段类型标注**——字段名 `api_key` 后紧跟冒号与类型名（`String`），其后是一段**没有 ASCII 空格的中文**。扫描器的「无引号赋值」规则要求「关键字 + `:`/`=` + ≥16 个非空格字符」，中文长句天然满足「非空格」条件。
- 对比：**已跟踪**扫描（`Invoke-ReleaseSecretScan` 的 worktree/index 分支）早就把该规则限定在配置/脚本路径（`$configOnlyPaths`：`*.json/yaml/yml/toml/sh/ps1/psm1/ini/conf/cfg/env/properties`），理由写在 `ReleaseBuild.Common.ps1:710-716`（在 Rust/JS 源码上会误报 `let secret = ...` 之类惯用法）。**只有 untracked 分支没有做同样的限定**，这就是不一致之处。

### 4.3 修法（最小、保留检测能力）

1. `Find-ReleaseSecretPatternFindings` 增加开关 `-SkipUnquotedAssignment`（默认行为不变，其它 3 处调用点不受影响）。
2. 新增单一事实源 `Get-ReleaseSecretAssignmentExtensions`（配置/脚本扩展名列表）与 `Test-ReleaseUnquotedSecretScanAppliesToPath`（含无扩展名 dotfile 如 `.env` 的判定）。
3. tracked 扫描的 `$configOnlyPaths` 改为由该列表派生（顺带补上 `.psd1`，消除"两处列表手工同步"的漂移风险）。
4. untracked 分支按路径判定后传开关：非配置/脚本文件跳过「无引号赋值」规则，**其余规则（私钥块、AWS key、`sk-` 形态、Bearer、authorization header、带引号赋值等）对所有文件照旧生效**，fail-closed 语义（读取失败、超 2 MiB、路径消失均 throw）未改。
5. 在既有 untracked 用例中加断言：`review-notes.md`（含 `api_key` + 类型名 + 长串无空格文本）不再触发；`local-build.env` 仍必须触发 throw；并直接钉住 helper 的四个路径判定（`.env`/`.ps1` = true，`.md`/`.rs` = false）。

### 4.4 验证

- 修复前（真实仓库）：`Invoke-ReleaseSecretScan -RepoRoot <repo>` → 抛错并列出上述 untracked 命中。
- 修复后（真实仓库，同一命令）：`OK: secret scan found no matches in Git-tracked or untracked build-input files.`
- 用例层：`Pipeline.Tests.ps1` 由 13/1 → **14/0**；untracked 用例仍以 `local-build.env` 触发 `untracked OpenAI-style API key`（fail-closed 覆盖未削弱）。
- 运行 D 全绿。

### 4.5 风险、边界与回退

- **有意收窄**：非配置/脚本的 untracked 文件里，形如「关键字 + 长值」的**无引号**赋值不再告警。带引号的真实凭据、各类高置信度形态（`sk-`、`AKIA`、私钥、Bearer、authorization）在该类文件中仍会被拦下；配置/脚本文件（真实密钥最常落点）保持全规则。
- 若 Lead 认为不应收窄：回退本项后，必须同时清理所有 untracked 非配置文件中此类文本，否则门禁第 2 步会继续失败——**回退与"保留评审产物"不可兼得**。
- 另一处同类隐患（**未改**，交给 Lead 决策）：evidence roots 扫描（`ReleaseBuild.Common.ps1:~880`）对所有文本文件套全规则，历史补丁靠目录级排除（`card-shell-cache`）压制同类误报；同一误报在 `.md` 形式的证据说明里仍可能出现。

---

## 5. G-12 定案：方向**确认写反**（留存的是 Windows 那份）

### 5.1 三重独立证据

1. **字节精确算术（本地可复现实验）**：Windows job 用 `Out-File -Encoding ascii -Append`（`release.yml:90-94`，`shell: pwsh` = PS 7 on Windows，行尾 CRLF），两个 Windows 产物文件名各 30 字符 → 每行 `64 + 2 + 30 + 2(CRLF) = 98`，两行 = **196 B**；实测 Release 上的 `SHA256SUMS.txt` = **196 B**。Android job 只有 1 个 APK（`find … | head -1` + 单个 `cp`，`:204-205`），`sha256sum` 单行 LF = `64 + 2 + 21 + 1 = 88 B`，恰等于 05:58 手工补传的 `SHA256SUMS-android.txt` = **88 B**。本地复现：`Out-File -Encoding ascii` 两行产出 **196 B / 2 条 CRLF**；模拟 `sha256sum` 单行产出 **88 B**。
2. **`ab894c6` 提交说明**（库内权威陈述）：「… so the release upload no longer silently overwrites one platform's checksums with the other's (**v0.1.1/v0.1.2 both shipped only the Windows sums**; v0.1.2 got SHA256SUMS-android.txt uploaded manually as remediation)」。
3. **补救动作指向 Android**：若 Windows 那份被覆盖丢失，人工补传的应是 Windows 校验和；实际补传的是 `-android`。

结论：同名冲突成立（两 job 都写裸名），但**实际留存并随发布分发的是 Windows 校验和，缺失的是 Android 校验和**（随后手工补传）。域4 曾据 `files:` 顺序推断「android 后写者胜」，经我回复字节级证据后已撤回；统一口径为「同名资产冲突下，后到的 Android 校验和未被接受/未留存」。

### 5.2 脚本侧防回退

上述 2.3(4) 断言：release.yml 中不得再出现裸名 `SHA256SUMS.txt`，且 `SHA256SUMS*` 唯一名 ≥ 2。（HEAD 现状：windows `:90-94` → `SHA256SUMS-windows.txt`；android `:204-205` → `SHA256SUMS-android.txt`。）

### 5.3 已应用的文档口径修正（**越出 task-17 写作用域的声明**）

task-17 写作用域不含 `docs/**`；但 task-17 描述明确「G-12 …按结论修脚本或**文档口径**（文档归 task-14）」，而本域同时是 task-14 的文档所有者，故直接落了两处最小修正（如需回退请告知）：

| 位置 | 改前（task-14 后状态） | 改后 |
| --- | --- | --- |
| `docs/RELEASE-STATUS.md:90` | 「…上传时会互相覆盖（v0.1.1/v0.1.2 均如此）；**覆盖方向…未能确认**…需取回文件原文才能定论，本文件不再断言方向」 | 「…上传时发生同名资产冲突…**2026-09-13 复核定案：留存的是 Windows 那一份，缺的是 Android 那份。**」+ 196/88 字节依据 + `ab894c6` 提交说明引文 |
| `docs/RELEASE-STATUS.md:30` | 「release.yml 的三个 job 只做构建与产物上传（…`npm ci` + `npm run build` + `cargo tauri build --ci`…）」 | 「以构建与产物上传为主（… + 一步前端契约测试 `node --test tests/tauri-command-contract.test.mjs`（2026-09-13 新增，未经真实 release 运行验证）+ `cargo tauri build --ci`…）+ Gitea 停用标记见 `.gitea/DECOMMISSIONED.md`」 |

依据：§5.1 三重证据；域4 通知的 T-10 终态（`release.yml:67-69` windows、`:156-158` android，各 1 步，未加整包 `npm test`）。`docs/RELEASE-STATUS.md` 行尾仍为纯 LF（CRLF=0，114 行），与该文件既有约定一致。

> 仍**未**回写（不在本轮任何作用域）：`.github/workflows/release.yml:247` 生成的 Release 正文模板 APK 通配串（域4 已改为 `*arm64-release.apk`，见其 T-10 第三处 hunk）；`docs/release-closure-2026-09-06.md:144` 的同一 G-12 方向表述 → 见 §7.4。

---

## 6. 逐条状态（G-07..G-20）

| ID | 07 号报告要点 | 本轮处置 | 状态 |
| --- | --- | --- | --- |
| G-07 | 发布工作流无 tag↔版本一致性校验 | `.github/**` 属域4 作用域 → §7.1 给出补丁草案 | 转交（未修） |
| G-08 | DOCS-CODE-AUDIT「175 命令全在 commands/*.rs」与代码不符（实际 156 + `card_studio_api.rs` 19） | 纯文档 → §7.2 给出建议文本 | 转交 task-16 |
| G-09 | ST-EVENTS-COVERAGE 事件常量数 32→实际 30、行号 20-51/53-59 → 29-60/62-68 | 纯文档 → §7.2 | 转交 task-16 |
| G-10 | ROADMAP Phase 6 状态行指向不存在的 HANDOFF §11.3 | 纯文档 → §7.2（并已核 §11.3 真实位置在 `BACKEND-ARCHITECTURE-SQLITE-CLOSURE-RESULT-2026-07-28.md:373`） | 转交 task-16 |
| G-11 | 第三次门禁运行未记录 | task-14 已修（`RELEASE-STATUS.md:34,39-40,93` 列三轮；仅第一轮含退出码行的口径已写明） | 已修（task-14） |
| G-12 | 校验和覆盖方向疑似写反 | §5 定案 + 断言 + 两处文档修正 | **已定案并修** |
| G-13 | 插件验收证据目录清单不全（缺 `channel-a/ channel-b/ windows/`） | task-14 已修（`RELEASE-STATUS.md:91`） | 已修（task-14） |
| G-14 | ROADMAP Phase 7 无状态标记与判定 | 替代证据已核（§7.3）→ 文档，转交 task-16，附建议状态行 | 转交 task-16 |
| G-15 | RELEASE-STATUS:56 把两轮复验指标合并成一句（4 类请求来自 rerun-1a；hookCalls=15 来自 rerun-1b） | 纯文档 → §7.2 建议标注轮次 | 转交 task-16 |
| G-16 | ROADMAP Phase 8 验收数字（212/21/422KB）为 07-08 时点且无注记 | 纯文档 → §7.2 建议加时点注记（当前值：node 504 / vitest 119 / `index-*.js` 603.22 kB） | 转交 task-16 |
| G-17 | 门禁数字无法用静态计数复现（口径说明） | 非缺陷；口径已固化在 07 号报告 §5 与 §3.4 | 无需修改 |
| G-18 | REGRESSION-COVERAGE:12「行号为文件当前位置」与 :4「行号会漂移」互相矛盾，且 `campaign.rs:389` 实际为 `:719` | 纯文档 → §7.2 | 转交 task-16 |
| G-19 | RELEASE-CHECKLIST:59-66 历史快照（2026-07-14，311/311、默认 JSON）与 :199（默认 SQLite）并存未标注 | 纯文档 → §7.2 | 转交 task-16 |
| G-20 | ARCHITECTURE-AUDIT:139-141 对 MVU 闭环的 06-17 结论已过期（`StubMvuRuntime` 仍在但生产路径已接通） | **该文件被 `.gitignore:83` 忽略**（属 Lead 发现的"27 个未入库 docs"问题） → §7.2 附时点注记建议 | 转交 task-16 |

---

## 7. 转交清单（可直接执行）

### 7.1 G-07：`.github/workflows/release.yml` 的 tag↔版本一致性校验（域4 / Lead）

建议在 `windows` job 的 `Install pinned tauri-cli` 之前插入一步（`.github/**` 不在本域作用域，未落盘）：

```yaml
      - name: Verify tag matches manifest version (fail closed)
        shell: pwsh
        run: |
          $tag = "${{ github.ref_name }}" -replace '^v', ''
          $cargo = (Select-String -Path Cargo.toml -Pattern '^version\s*=\s*"([^"]+)"' | Select-Object -First 1).Matches[0].Groups[1].Value
          $conf = (Get-Content crates/tauri-app/tauri.conf.json -Raw | ConvertFrom-Json).version
          if ($tag -ne $cargo -or $tag -ne $conf) { throw "tag $tag != Cargo.toml $cargo / tauri.conf.json $conf" }
```

责任边界：补丁由能改 `.github/**` 的一方落盘；验证方法 = 打一个预发布 tag 或 `workflow_dispatch` 干跑（**未经真实运行不得写成已验证**）。

### 7.2 纯文档条目：原文本 → 建议文本（task-16）

| ID | 文件:行 | 建议改法 |
| --- | --- | --- |
| G-08 | `docs/DOCS-CODE-AUDIT.md:11`（并 `:28`、`:54`） | 「175 个命令分布于 `crates/tauri-app/src/commands/*.rs`（156）」→ 改为「`commands/` 156 个 + `crates/tauri-app/src/card_studio_api.rs` 19 个 = 175 个，全部注册于 `crates/tauri-app/src/lib.rs` 的 `generate_handler!`」 |
| G-09 | `docs/ST-EVENTS-COVERAGE.md:11`、`:48` | `32 个` → `30 个`；`plugin-bridge.js:20-51` → `:29-60`；`ST_EVENT_ALIASES … :53-59` → `:62-68`；并加「截至 2026-09-13 复核」 |
| G-10 | `docs/ROADMAP.md:149` | 「见 `docs/HANDOFF.md` §11.3」→ 「见 `docs/workstreams/BACKEND-ARCHITECTURE-SQLITE-CLOSURE-RESULT-2026-07-28.md` §11.3」 |
| G-15 | `docs/RELEASE-STATUS.md:56` | 把「4 类上游请求」标为 `rerun-1a`（第 3 轮 4 连调 + `r3-proxy-analysis.json`），把「hookCalls=changes=15、events=20、0 DataCloneError」标为 `rerun-1b` |
| G-16 | `docs/ROADMAP.md:210-211` | 数字后加「（2026-07-08 时点；当前计数见 RELEASE-STATUS：node 504 / vitest 119 / 构建 `index-*.js` 603.22 kB）」 |
| G-18 | `docs/REGRESSION-COVERAGE.md:12` | 删「行号为文件当前位置」或改为「以测试函数名检索为准（行号会漂移）」；`:14` 的 `campaign.rs:389` 更正为 `:719`（`resolved_persona` 本体在 `:384`） |
| G-19 | `docs/RELEASE-CHECKLIST.md:59` | 标题改为「### 2026-07-14 已验证（历史快照，非当前状态；代码基线 `99b1ea3`）」，并保留 `:199` 的 SQLite 现状态 |
| G-20 | `docs/ARCHITECTURE-AUDIT.md:139-141`（**被 ignore 文件**） | 第 5 节加时点注记：「2026-06-17 时点判断；MVU runtime 已于 W10 接通（`docs/ROADMAP.md:126`，生产路径 `crates/tauri-app/src/mvu_webview_runtime.rs`），插件权限分层仍未覆盖」；并纳入 Lead 的"本地工作笔记 vs 入库权威文档"声明 |

### 7.3 G-14：ROADMAP Phase 7 判定（task-16 用）

替代证据核查结果（本域核对，未改文档）：

| Phase 7 任务（`ROADMAP.md:175-180`） | 现有证据 | 建议判定 |
| --- | --- | --- |
| 端到端验收矩阵 | `docs/REGRESSION-COVERAGE.md`（169 行）、`docs/RELEASE-CHECKLIST.md` 存在 | 部分（未在 ROADMAP 汇总成矩阵） |
| 回归测试与 LLM 质量评测样例 | 门禁日志 node 504 / vitest 119；Gate 6 四项 PASS + seal（`harness-real-llm`）；Full100 未 seal | 部分（长程样例未闭合） |
| 备份/迁移/恢复/排障 bundle | `release-closure-2026-09-05/android-database-comparison.json`（20 表 identical、`schema_migrations 8→8`）；Gate 8 封存 RESULT §37 | 达成 |
| 长会话性能、Agent 调用成本、移动端稳定性 | 移动端稳定性 = 真机 9 环节（`artifacts/realmodel-2026-09-06/android/summary.md`）；性能/成本仅有单轮 33 次调用延迟中位（`windows/summary.md`）与未 seal 的 100-turn endurance | **无长会话专项证据**（建议标「无法判定/无专门证据」） |
| 桌面/Android 发布包 + 首次使用文档 | v0.1.2 Release 三资产；`docs/USER-GUIDE.md`（231 行） | 达成 |
| 下一阶段战略与不做事项 | `docs/PLAN-POST-MAINLINE.md`（12,110 B）；`RELEASE-STATUS.md:11` 明确排除项 | 达成 |

验收三条：①「新用户能按文档完成第一局 Campaign」**无用户实操记录 → 无法判定**；②「测试者能按 checklist 完成发布前检查」达成（`RELEASE-CHECKLIST.md` + 三轮 11 步门禁）；③「有明确 post-mainline 优先级」达成。
建议在 `ROADMAP.md:169` 下补一行：`**状态：部分完成**（端到端矩阵/长会话性能与成本项无专门证据；验收①无实操记录，②③已满足）——逐项依据见 docs/review-2026-09-13/07-goals-and-claims.md 与 fixes/09 §7.3`。

### 7.4 其它转交

- `docs/release-closure-2026-09-06.md:144` 的 G-12 方向表述仍是旧方向（「android 覆盖 windows」）→ 建议与 `RELEASE-STATUS.md:90` 统一为「留存 Windows、缺 Android（后补传）」；该文件不在本轮作用域，交 task-16。
- `.gitea/DECOMMISSIONED.md` 需入库（当前 untracked）——`git add` 由 Lead 统一处理。
- `.gitignore` 的 27 个未入库 docs（含 `CLAUDE.md` 必读第 1/3 项）→ 本轮按 Lead 决定**未动**。

---

## 8. 不可本地验证项：取证步骤与责任边界

| 项 | 现状证据 | 需要的取证步骤 | 责任边界 |
| --- | --- | --- | --- |
| Gitea Actions 停用（act_runner 停止、run 34-36 取消、`has_actions` 关闭） | 仅文档自述（`RELEASE-STATUS.md:30,92`、`.gitea/DECOMMISSIONED.md`）；服务端状态无法离线核实 | 用管理员账号登 Gitea `git.2529985.xyz/ss/story` → 仓库 Settings → Actions（确认未启用）；Admin → Actions/Runner 列表（确认 act_runner 离线）；仓库 Actions 页（确认 run 34-36 为 canceled/未执行）；截图或 JSON 存档 | 仓库所有者/用户（本机无凭据、无网络出口） |
| APK 为 release keystore 签名 | workflow 与脚本存在（`.github/workflows/release.yml:161-187`、`scripts/ci-patch-android-signing.py`）；run `34013739416` success；但无 `apksigner verify` 输出 | ① `apksigner verify --print-certs app-arm64-release.apk`（比对证书主体/ fingerprint 与 release keystore）；② `keytool -printcert -jarfile app-arm64-release.apk`；③ 拉取 run `34013739416` 的 android job 日志，确认 `keystore bytes:` 与 signingConfig patch 步骤 | 持有 release keystore 与 GitHub 权限者；产出应为一份可离线复算的 `apksigner` 输出文本，落 `artifacts/release-closure-*` |
| v0.1.2 三产物「下载实测」过程 | 哈希数值与 GitHub 存储 digest 一致（可核）；「下载并实测」动作本身无日志 | `gh release download v0.1.2 -D <dir>` → `Get-FileHash -Algorithm SHA256`（或 `sha256sum -c SHA256SUMS*.txt`）→ 与文档哈希 `75bfc0c5…`（APK）/`7759fece…`（EXE）/`df8bf8c2…`（MSI）比对，留存命令输出 | 任何具备网络与 `gh` 的成员；本轮本机网络出口受限 |

---

## 9. 改动文件与校验

| 文件 | 变更量 | 说明 | 入库 |
| --- | --- | --- | --- |
| `scripts/tests/ReleaseBuild.CI.Tests.ps1` | +71/−6 | G-06 标注与断言、G-12 防回退、untracked 假阳性回归断言、legacy 枚举限定 YAML | 需 `git add` |
| `scripts/release-build/ReleaseBuild.Common.ps1` | +54/−9 | legacy docstring；`Find-ReleaseSecretPatternFindings -SkipUnquotedAssignment`；扩展名单一事实源 + 路径判定 helper；untracked 分支作用域 | 需 `git add` |
| `scripts/tests/ReleaseBuild.RunnerReadiness.Tests.ps1` | +8/−1 | Describe 更名 + LEGACY 注释（内部 198 项中的 99 项断言未改） | 需 `git add` |
| `.gitea/DECOMMISSIONED.md` | 新增 2,720 B | 停用标记 | 新增，untracked |
| `docs/RELEASE-STATUS.md` | +20/−12（与 task-14 同一批变更行） | §5.3 两处口径修正（G-12 定案 / 远端 CI 范围 + 标记指针） | 已跟踪，需同批提交 |

校验命令与结果：

- `git status --porcelain -- scripts .gitea` → ` M scripts/release-build/ReleaseBuild.Common.ps1`、` M scripts/tests/ReleaseBuild.CI.Tests.ps1`、` M scripts/tests/ReleaseBuild.RunnerReadiness.Tests.ps1`、`?? .gitea/DECOMMISSIONED.md`（`scripts/architecture/backend-baseline.mjs` 的改动属 task-11，本域未触碰）
- `git diff --numstat`（本域 4 文件）→ `54/9`、`71/6`、`8/1`、`20/12`
- ASCII 守卫（`scripts/**/*.ps1` 全 ASCII）：三个 .ps1 非 ASCII 字节均为 **0**（`.gitea/DECOMMISSIONED.md` 亦为 0）
- `It` 计数：`51 / 14 / 34 / 99 = 198`（与基线一致）
- 行尾：三个 .ps1 与其同类未改文件一致（纯 CRLF，`core.autocrlf=true` 检出态）；`docs/RELEASE-STATUS.md` 与 `.gitea/DECOMMISSIONED.md` 纯 LF；无混合行尾
- Pester：`Release build tests passed.`，退出码 0（§3 运行 D）

---

## 10. 需 Lead 决策 / 遗留风险

1. **§4 的收窄是否接受**：非配置/脚本的 untracked 文件不再套「无引号赋值」规则。若要求保持原覆盖，则必须清理所有 untracked 非配置文件中的同类文本，否则门禁第 2 步会继续失败（回退与保留评审产物不可兼得）。
2. **evidence roots 的同类误报**：**已在 task-24 收敛**（复用同一路径判定，见 §11）；带引号赋值规则的口径按 Lead 决定**不扩大**。
3. **§5.3 越界修正**：`docs/RELEASE-STATUS.md` 两处属 task-17 写作用域之外（由 task-14 文档所有者执行），如不接受请回退并把结论转给 task-16。
4. **入库**：`.gitea/DECOMMISSIONED.md` 为新文件；三份 `scripts/**` 改动需提交，否则当前树上的 Pester 结果不代表 HEAD。
5. **task-16 派发**：§7.2/§7.3 的清单可直接作为 task-16 的验收项；`docs/release-closure-2026-09-06.md:144`（G-12 旧方向）与 `docs/ST-EVENTS-COVERAGE.md`、`docs/RELEASE-CHECKLIST.md`、`docs/ROADMAP.md`（G-10/G-14/G-16）一并纳入。
6. **未验证项不得升级为结论**：远端 CI 新增的 `node --test` 契约测试步骤未经真实 release 运行；Gitea 服务端状态、APK 签名、下载实测三项仍按 §8 标注为"未独立验证"。

---

## 11. task-24 追加：evidence roots 扫描同类误报的核查与收敛

- 任务：**task-24**（承接 §10.2）；写作用域：`scripts/**`、本文件、`artifacts/review-2026-09-13/**`（仅新增日志）
- 结论：**会误报（已复现）→ 已按与 task-17 相同的路径判定收敛**；带引号赋值规则的口径按 Lead 决定**不扩大**（避免削弱对真实 JS 泄漏的覆盖）。

### 11.1 复现（改前）

命令：`Invoke-ReleaseSecretScan -RepoRoot <repo> -EvidenceRoots <root>`（调用点仅 `scripts/verify-release.ps1:158-160`：传 `-EvidenceRoot` 或设 `STORYFORGE_EVIDENCE_ROOT` 时才走 evidence 分支）

| 证据根 | 改前结果 |
| --- | --- |
| `docs/review-2026-09-13`（真实评审产物） | THROW — `evidence unquoted secret assignment at docs/review-2026-09-13/01-domain-infra.md` |
| 合成探针：`notes/review.md`（`api_key` + 类型名 + 无空格中文）、`build.env`（`api_key=` + 32 个 `A`） | THROW — 同时列出 `notes/review.md`（误报）与 `build.env`（应报） |
| `artifacts/review-2026-09-13`（门禁日志） | 自身无误报（内容不含触发形态） |

即：**误报类与 task-17 的 untracked 完全相同**，只差触发路径（证据根 vs 工作树）。

### 11.2 改法（复用同一判定，不复制第二套逻辑）

`ReleaseBuild.Common.ps1` 的 evidence 分支：

```powershell
$skipUnquotedAssignment = -not (Test-ReleaseUnquotedSecretScanAppliesToPath -RelativePath $file.Name)
$patternHits = @(Find-ReleaseSecretPatternFindings -Text $text -SkipUnquotedAssignment:$skipUnquotedAssignment)
```

未改动（fail-closed 与覆盖面均保持）：其余全部规则照旧对所有文本文件生效；二进制扩展名跳过、`card-shell-cache` 目录排除、单文件 >10 MiB 跳过、不可读跳过、证据根不存在即 throw、发现命中即整体 throw。

### 11.3 改后实测（可离线核查的原始输出：`artifacts/review-2026-09-13/evidence-roots-scan-task24.log`）

- **规则级 before/after**（同一文件 `docs/review-2026-09-13/01-domain-infra.md`）：`Find-ReleaseSecretPatternFindings`（全规则）→ `secret-pattern:unquoted secret assignment`；加 `-SkipUnquotedAssignment` → **空**。
- **真实证据根**：`docs/review-2026-09-13` → `PASS`；`artifacts/review-2026-09-13` → `PASS`。
- **合成探针**：仅 prose `.md` → `PASS`；加入 `build.env` → `THROW`（`evidence unquoted secret assignment at …\build.env`，fail-closed 保留）。

### 11.4 防回退断言

- `scripts/tests/ReleaseBuild.CI.Tests.ps1` → `Describe 'ReleaseBuild secret scan untracked inputs'` → `It 'scans untracked build-input files and fails closed without echoing secrets'`（既有用例内新增分段；`It` 计数仍 34，总计数仍 198）。
- 新增断言：evidence root 内 `summary.md`（prose）→ `Should Not Throw`；同一 root 内 `captured.env`（`api_key=` + 32×`A`）→ `Should Throw`；随后删除该 `.env` 以免影响后续断言。
- 保留 task-17 的路径判定断言：`local-build.env` / `scripts/build.ps1` → `$true`；`docs/review-notes.md` / `src/main.rs` → `$false`。

### 11.5 Pester 复跑

| 运行 | 前置条件 | 结果 |
| --- | --- | --- |
| A | 外部临时文件 `frontend/tests/_m24_selfcheck.cjs` 仍在 | `Tests.ps1` 51/0、`Pipeline.Tests.ps1` 13/**1**（唯一失败 = `default repository secret scan passes without an allowlist`，根因见 11.6）→ fail-fast 停止 |
| B | 遗留文件已被域5 清理 | **51 / 14 / 34 / 99 = 198 通过、0 失败、0 跳过**，末行 `Release build tests passed.`，退出码 **0** |
| C（最终，含 11.7 的脚本修正） | 同上 | **51 / 14 / 34 / 99 = 198 通过、0 失败、0 跳过**，末行 `Release build tests passed.`，退出码 **0** |

运行 C 的完整日志：`artifacts/review-2026-09-13/pester-task24-review-goals.log`。

### 11.6 临时文件事件（证据链，按 Lead 要求记录）

- **现象**：2026-09-13 深夜 `Invoke-ReleaseSecretScan` 对所有调用报 `untracked secret assignment at frontend/tests/_m24_selfcheck.cjs`，使门禁第 1 步（secret scan）与第 2 步（Pester 的 `default repository secret scan passes without an allowlist`）**必然变红**。
- **对象**：`frontend/tests/_m24_selfcheck.cjs`（untracked、3,206 B、mtime 2026-09-14 01:07:51），首行自述「临时自检（不属于交付物，运行后立即删除）」；命中的是**带引号赋值**规则——第 58 行的 JS 字符串比较（`'token:'`）被规则当成"键值赋值"，属源码误报。同目录 `frontend/tests/_m31_selfcheck.mjs` 存在但不触发。
- **处置**：本域**未动** `frontend/**`；Lead 判定归域5 立即清理，并明确**不扩大带引号规则口径**（该规则在 tracked 扫描中本就覆盖全树，收窄属策略变化）。域5 已删除两份临时文件（`Test-Path` True → False），其后运行 B/C 全绿。
- **结论**：该红灯与本域改动无关，但证明**工作树中的临时/未跟踪文件可以单独把门禁打红**；建议后续自检脚本一律放 `artifacts/**`（该目录已在扫描排除清单内）。

### 11.7 顺带修正：`scripts/run-real-card-smoke.ps1` 默认夹具路径（域6 请求，属本域写作用域）

review-meta-plugin（域6）反馈：脚本默认指向仓库根 `test-card.png`，而本机实际夹具在 `data/local/test-card.png`，脚本直接抛 "Complex card fixture not found"；域6 侧 Rust 默认已改为同一路径并有守卫测试。

| 位置 | 改前 | 改后 |
| --- | --- | --- |
| `:2` | `[string]$FixturePath = "test-card.png"` | `[string]$FixturePath = "data/local/test-card.png"`（与 `crates/infra-import/src/lib.rs default_real_card_fixture_path()` 一致；`data/` 被 `.gitignore:18` 忽略、CI 中缺失，故该 smoke 仍是本地 opt-in） |
| `:57-79` | 单一候选路径 + 报错「place test-card.png at the repository root」 | 候选解析：显式 `-FixturePath`（绝对/相对）→ 未显式传参时先 `data/local/test-card.png`、再回退仓库根 `test-card.png`；两者都不存在时报错写明两个默认位置，且**不复述绝对路径**（保持原脚本 `Fixture path withheld` 的隐私约定） |

校验：AST 解析 0 错误；`scripts/**/*.ps1` 全 ASCII 守卫不受影响（非 ASCII = 0）；默认路径实测解析到 `data\local\test-card.png`；`scripts/tests/**` 中**没有**任何断言引用该脚本或该默认值（grep 无命中），Pester 计数仍 198。
**转交（不在本域写作用域）**：`docs/RELEASE-CHECKLIST.md:72`、`:136`（「默认读取仓库根目录 `test-card.png`」）、`docs/PLAN-ST-IMPORT-EXPORT.md:127`、`docs/DOCS-CODE-AUDIT.md:300` 与新默认值不一致 → 建议并入 task-16 统一回写（`DOCS-CODE-AUDIT.md:300` 是 2026-07-07 的历史核对记录，可保留原文）。

---

## 12. task-31 追加：R8 收口（M-30 文档漂移 + G-07 tag↔版本校验）

- **任务**：task-31（owner `review-goals`），收口 R6 §2「无记录」里归属本任务的两条：M-30、G-07。
- **上游**：`round2/R6-fix-completeness-audit.md` §2；本节为 R8（`round2/R8-unrecorded-closure.md`）的记录侧。
- **写作用域**：本文件 + `docs/ROADMAP.md` / `docs/PLAN-ST-IMPORT-EXPORT.md` / `docs/RELEASE-STATUS.md` + `scripts/release-build/**` + `scripts/tests/**` + `.github/workflows/release.yml`；**未改** Rust/前端代码、`.gitignore`、其他域 fixes 记录、`round2/R1..R7*.md`。

### 12.1 状态摘要

| 发现 | 原状态（本文件 §6） | 现状态 | 依据 |
| --- | --- | --- | --- |
| M-30（P3，3 子项） | 跨域承接（task-f7），下游无任何处置记录 | **已修复（文档）** | §12.2–§12.4 |
| G-07（P2） | 转交（未修） | **已修复（脚本 + Pester 用例 + release.yml 步骤；CI 侧未运行时验证）** | §12.5–§12.7 |

### 12.2 M-30-① `CLAUDE.md` 的 `card_shell_clear_cache`「UI 入口未接」

| 项 | 原文本 | 现状/依据 |
| --- | --- | --- |
| `CLAUDE.md`（card-shell 清尾 L6） | 「`card_shell_clear_cache` 命令 + wrapper（UI 入口未接）」 | **已修复（2026-09-13, task-16）**：现文写「**UI 入口已接线**（2026-09-13 更正）：`frontend/src/components-v2/shell/InspectorDrawer.vue`」 |

**本轮代码复核（不引用记录）**：`InspectorDrawer.vue:18` `import { cardShellClearCache } from '../../tauri-api.js'`、`:45` `const n = await cardShellClearCache()`；wrapper 在 `frontend/src/tauri-api.js:1075`。结论：task-16 的回写正确，本轮**未再改** `CLAUDE.md`，仅在 R8 标注。

### 12.3 M-30-② bundle `format_version` 口径（`ROADMAP.md:123` + `PLAN-ST-IMPORT-EXPORT.md:62/63/111`）

**代码事实（全链路）**：`commands/import_export.rs:22` 常量 `BUNDLE_FORMAT_VERSION = 2`（JSON 导出写于 `:603`，该路径 `runtime: None`）；SQLite/后端导出挂上 `runtime` 后写 **3**（`storage_backend.rs:1554/:1562`，并 `:1563` 立即 `validate_runtime`；`sqlite_runtime.rs:278` `if runtime.is_some() { 3 } else { 2 }`）；导入接受 **1..=3**（`:654` 与 `:1047` 拒绝 `0`/`>3`）；**v3 必须有 `runtime`**，否则 `commands/bundle_runtime.rs:20-23` 报 `v3 Bundle 缺少正文快照`。测试锚点：`tests/sqlite_character_lifecycle.rs:397`（=3）/`:576`（=2）。
**判定：代码自洽**——常量 2 只描述"无正文快照"这条导出路径，v3 是有正文快照的路径；**无代码缺陷，不需要 Lead 决策**，问题在文档侧。

| 位置 | 原文本 | 新文本（要点） |
| --- | --- | --- |
| `docs/ROADMAP.md:123` | `- ~~设计 StoryForge Campaign 导出格式。~~ ✅ JSON bundle（`format_version`）` | 追加「2026-09-13 更正（task-31，M-30-②）」：导出 v2（无正文，`import_export.rs:22/:603`）/ v3（含正文，`sqlite_runtime.rs:278`）；导入接受 1..=3 且 v3 必须带 `runtime`（`bundle_runtime.rs:20`） |
| `docs/PLAN-ST-IMPORT-EXPORT.md:62` | `…专有 JSON Bundle v2，包含完整 CharacterCard…` | 改为「JSON Bundle v2（无正文快照）…」并补 v2/v3 双路径与行号 |
| `docs/PLAN-ST-IMPORT-EXPORT.md:63` | `…v2 使用 bundle 内的完整 CharacterCard，兼容 v1 只有 definitions 的旧 bundle…` | 改为「接受 `format_version` 1..=3 …v3 必须带 `runtime` 正文快照（缺失即 validation）」 |
| `docs/PLAN-ST-IMPORT-EXPORT.md:111` | `- [x] 版本号和向前兼容策略：`format_version = 2`；导入兼容 v1（无完整 card，仅 definitions）。` | 改为导出 2/3 双路径 + 导入 1..=3 三档语义 + `0`/`>3` fail-closed |

### 12.4 M-30-③ `RELEASE-STATUS.md:29`「第三方插件两条通道闭合」边界限定

- **原文本（节选）**：`…1A manifest 插件 4 个真实缺陷修复并复验。边界与证据见下节 |`
- **新文本（仅追加，原声明保留）**：`…边界与证据见下节。**边界限定（2026-09-13，task-31／M-30-③）**：「通道闭合」指已验证的挂载／执行／权限门控路径在验收样本上闭合，**不等于不存在可绕过路径**——子帧／权限边界的运行时验证见 P0-2（`M-01`：Windows 子帧 iframe 仍持有 Tauri IPC，本轮暂缓未修）；本行结论不构成插件沙箱攻击面评估（另见本文件 `:60` 的免责声明）。原「已闭合」判定不改写 |`
- **依据**：`M-01`（P0-2）状态为"暂缓（主体未修）"（`06-meta-plugin-fixes.md` 结论 1、R6 §7）；`:60` 已有"不等于密码学审计或插件沙箱攻击面评估"免责声明。

### 12.5 G-07：新增 `scripts/release-build/verify-tag-version.ps1`（可离线测试）

- **输入**：`-Tag <tag>`（必需）、`-RepoRoot <path>`（默认 `git rev-parse --show-toplevel`）、`-PassThru`。
- **版本来源**：`crates/tauri-app/tauri.conf.json` 的 `"version"` 与根 `Cargo.toml` `[workspace.package] version`（两源必须一致；`crates/tauri-app/Cargo.toml` 为 `version.workspace = true` 继承前者）。
- **规则**：tag 可带 `v/V` 前缀（剥离后比较，无前缀也接受）；必须是 `MAJOR.MINOR.PATCH[-prerelease]`；数字核心必须等于 manifest；manifest 无 prerelease 时 tag 可带（`v0.1.2-rc.1` ↔ `0.1.2`），manifest 自带 prerelease 时 tag 必须写同一整版；文件缺失/不可解析/两源漂移一律 fail-closed。
- **退出码**：`0` 匹配 / `1` 不匹配·漂移·不可解析·缺文件 / `2` 用法错误。
- **编码**：ASCII-only、CRLF、无 BOM（与 `scripts/**` 既有约定一致；PowerShell 5.1 与 7 均可）。
- **冒烟（真实仓库）**：`v0.1.2` → exit 0；`v0.1.3` → exit 1（`FAIL tag version 0.1.3 does not match manifest version 0.1.2`）；无 `-Tag` → exit 2。

### 12.6 G-07：Pester 用例与 `release.yml` 步骤

- **新增套件**：`scripts/tests/ReleaseBuild.TagVersion.Tests.ps1`（10 用例）。必需四类：匹配（`v0.1.2`）、无 v 前缀（`0.1.2`）、预发布后缀（`v0.1.2-rc.1`）、不匹配（`v0.1.3`）；补充六类：两源漂移、tag 非法（`v1.2`）、manifest 自带 prerelease 时 tag 必须重复、版本来源缺失、缺 `-Tag` → exit 2、真实仓库当前版本自洽。
- **已接入** `scripts/tests/run-release-build-tests.ps1` 的 `$testFiles`（现第 5 个文件，`:59`），因此进入 11 步门禁的 Pester 步骤。**无脚本硬编码 198**（`Select-String '\b198\b' scripts/**, .github/**` 无命中），新增用例不会撞断言；本文件 §11.7 记录的「Pester 计数仍 198」是 task-24 时点值，task-31 后为 **208**。
- **`release.yml` windows job 新增步骤（构建前）**：

```yaml
      # G-07 (review-2026-09-13): fail closed before building when the pushed tag
      # disagrees with the application version in tauri.conf.json / Cargo.toml.
      # workflow_dispatch runs are branch-based, so this check is tag-only.
      - name: Verify tag matches app version (fail closed)
        if: startsWith(github.ref, 'refs/tags/')
        shell: pwsh
        run: pwsh -NoProfile -File scripts/release-build/verify-tag-version.ps1 -Tag '${{ github.ref_name }}'
```

- 位置：`Install pinned tauri-cli` 之后、`Build Windows bundle (msi + nsis)` 之前（当前 `:80-86`）。`workflow_dispatch` 的 `github.ref_name` 是分支名，故用 `if: startsWith(github.ref, 'refs/tags/')` 门控（无门控会误失败）。
- 不发布保证：`release` job `needs: [windows, android]`（`:228`）+ `if: startsWith(github.ref, 'refs/tags/')`（`:230`），windows job 失败即不发布。
- **契约影响：未破**——加步后 `ReleaseBuild.CI.Tests.ps1` 的 99 条静态契约（release.yml job 头、`cargo tauri build --ci`、`SHA256SUMS-*` 命名、`tauri-cli --locked --version`/`2.11.2`/`SkipBundle` 等断言）**全部通过**，因此**未修改任何契约测试**（不存在"最小契约更新"这一步，走的是要求里的"加步即可"路径）。
- **CI 侧改动未运行时验证**：本地仅做静态契约 + YAML 解析 + 脚本离线用例；真实 tag push 的执行证据待下一次发布。

### 12.7 验证（真实数字）

| 校验 | 命令 | 结果 |
| --- | --- | --- |
| 发布构建测试全量 | `powershell -NoProfile -ExecutionPolicy Bypass -File scripts/tests/run-release-build-tests.ps1` | **exit 0**；`Passed: 51 / 14 / 34 / 99 / 10`，**合计 208 / Failed 0 / Skipped 0**（原 198） |
| workflow YAML | `Test-ReleaseWorkflowSyntax -Path .github/workflows/release.yml` | pyyaml，`Valid=True`、`ErrorCount=0` |
| 脚本冒烟 | `verify-tag-version.ps1 -Tag v0.1.2 / v0.1.3 / 无参数` | exit `0 / 1 / 2` |
| 编码约定 | 两个新 `.ps1` | 非 ASCII=0、CRLF、无 BOM |

### 12.8 改动文件清单

新增 `scripts/release-build/verify-tag-version.ps1`、`scripts/tests/ReleaseBuild.TagVersion.Tests.ps1`、`docs/review-2026-09-13/round2/R8-unrecorded-closure.md`；修改 `scripts/tests/run-release-build-tests.ps1`（`$testFiles` +1 行）、`.github/workflows/release.yml`（+7 行步骤）、`docs/ROADMAP.md`、`docs/PLAN-ST-IMPORT-EXPORT.md`、`docs/RELEASE-STATUS.md`、本文件（§12）。
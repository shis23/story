# 域7 审查报告：预期目标与声明复核（文档 / 门禁 / harness / CI / 证据）

> 审查者：review-goals（声明审计员）
> 审查日期：2026-09-13
> 代码基线：`main@ab894c6`（`git log -1` 输出 `ab894c6 fix(release): platform-suffixed checksum files; record v0.1.2 closure`；`git status --porcelain` 为空）
> 方法：只读核对。允许并实际使用的工具：`read/grep/glob`、`git log/show/diff/tag/for-each-ref`、`Test-Path/Get-ChildItem/Select-String`、`ConvertFrom-Json`、GitHub 公开 API（`api.github.com`）与本地证据文件。
> 未执行：`cargo` / `npm` / `verify-release.ps1` / 任何构建、测试、发布脚本（由 Lead 统一跑门禁）。
> 原则：每条发现给 `文件:行号` 或命令+输出；数量类事实给统计命令与结果；区分「已核实（高置信）」与「疑似（中/低置信）」；不用文档证明文档。

---

## 1. 范围与覆盖率

### 1.1 已覆盖的文档（全部逐节阅读或定向核对）

| 文件 | 行数 | 覆盖方式 |
| --- | --- | --- |
| `README.md` | 104 | 全文 |
| `docs/ROADMAP.md` | 224 | 全文（Phase 1–8 + 后注记） |
| `docs/RELEASE-STATUS.md` | 106 | 全文 |
| `docs/ARCHITECTURE.md` | 188 | 全文 |
| `docs/HANDOFF.md` | 147 | 全文 |
| `docs/DOCS-CODE-AUDIT.md` | 381 | 头部增量 + 定向检索（首 303 行读取 + 关键行定位） |
| `docs/RELEASE-CHECKLIST.md` | 199 | 头部/基线段 + 存储段 + L7/L8 行 |
| `docs/release-closure-2026-09-06.md` | 157 | 全文 |
| `docs/ST-EVENTS-COVERAGE.md` | 184 | 第 1–74 行 + 代码对照 |
| `docs/REGRESSION-COVERAGE.md` | 169 | 头部 + 定向检索 |
| `docs/USER-GUIDE.md` | 231 | 存储声明定向检索 |
| `docs/ARCHITECTURE-AUDIT.md`（CLAUDE.md 必读第 3 位） | 164 | 状态行/结论/待执行段 + 代码对照 |
| `docs/DATA_MODEL.md`（CLAUDE.md 必读第 5 位） | 209 | 关键声明定向检索（变量/`tool_whitelist`/默认 profile） |
| `docs/FRONTEND-COMPONENTS.md` / `docs/AGENT_INTERFACES.md` / `docs/INTENT.md` / `docs/TECHNICAL_DESIGN.md` | 283/286/39/37 | 存在性 + 定向检索 |
| `docs/workstreams/**`（41 个活跃文件） | — | 目录 + `BACKEND-ARCHITECTURE-SQLITE-CLOSURE-RESULT-2026-07-28.md` §11.3/§12.1/§35/§36/§37 定向 |
| `scripts/**`（24 个文件）、`.github/workflows/release.yml`、`.gitea/workflows/*` | — | 全部脚本头部/关键函数；`verify-release.ps1` 全文 |
| `crates/harness-real-llm/**`（42 文件 / 34,597 行，含空行） | — | 目录 + seal/ignored 定向 |
| `artifacts/**` | — | 存在性、大小、尾部/摘要文件（未读超大日志全文） |
| `C:\Users\Predator\storyforge-evidence\gate6-2026-08-02\**` | — | 4 份 `run_manifest.json` 内容 + 4 个 full run 目录存在性 |

### 1.2 未覆盖/无法覆盖

- 未复跑任何门禁；Rust/前端/Pester 计数只能由已存日志 + 静态计数交叉核对（口径见 §5）。
- Gitea 服务端状态（`has_actions`、act_runner 是否停止、run 34–36 是否取消）无本地可核证据，标记「无法独立验证」。
- Android APK 的 release keystore 签名断言无本地可核的 `apksigner verify` 证据（见 §6 D2）。
- 未下载 release 资产正文（仅用 GitHub API 的 size/digest 字段）；`SHA256SUMS.txt` 内容未取回（见 G-12，属疑似）。

---

## 2. 结论摘要

### 2.1 预期目标完成度总评

| 维度 | 结论 |
| --- | --- |
| 本次既定 v0.1.2 发布范围（Campaign 写作 / 三种模式 / 采纳与快照边界 / ST 导入导出 / Meta-MVU / 默认 SQLite / Windows+Android 真机 / 第三方插件 / 分发安装包 / 远端 CI） | **基本达成，且核心结论可独立核实**。14 行门禁表中 12 行达成、1 行部分达成、1 行（「源码和文档一致性」）不成立。未发现把未达成或失败写成已达成的虚假验收（**P0 = 0**）。 |
| 声明体系一致性（README / ARCHITECTURE / HANDOFF / ROADMAP 与当前事实） | **未达成**：收尾（09-05/09-06）后 4 份「当前」文档未回写，产生 5 条 P1 级冲突或失效声明；`ROADMAP Phase 7` 至今无状态判定。 |
| 未完成/受限项是否如实标注（Gate 6 / Full100 / 记忆参数 / CoT / JSON 写路径 / ST 99） | **达成（保守口径）**。全部未完成项均明确标注为未完成或非 PASS，且我在本机找到了与之相符的负面证据（如 4 个 `run-full-*` 目录无 `run_manifest.json`）。仅 1 项（Windows runner）在跨文档层面未收敛。 |
| 证据可复现性 | **部分达成**：门禁计数可用三轮日志复现且三轮一致；Gate 6 seal 证据在本机存在且时间戳与文档逐条吻合；但存在 1 次未记录的完整门禁运行（G-11）、1 处证据清单不全（G-13）、1 处指标跨轮合并（G-15）。 |

### 2.2 逐条核对统计（39 项，明细见 §4）

| 判定 | 数量 | 说明 |
| --- | --- | --- |
| 达成 | **29** | 含 ROADMAP 6 / README 7 / 门禁表 12 / ARCHITECTURE 2 / HANDOFF 2 |
| 部分达成 | **5** | ROADMAP Phase 6、门禁表「ST 世界书往返」、ARCHITECTURE 技术债 3/4、HANDOFF 第 4 项 |
| 未达成 | **4** | README 第 8 条（陈旧冲突）、门禁表第 1 行（文档一致性不成立）＝**虚假声明类**；HANDOFF 第 3/5 项（JSON 写路径删除、CoT 三臂）＝**如实标注类** |
| 无法判定 | **1** | ROADMAP Phase 7（无状态、无判定；其中性能/成本项无专门证据） |

### 2.3 发现计数

| 严重度 | 数量 | ID |
| --- | --- | --- |
| **P0** | **0** | —（无虚假验收；若评审把「发布物说明不成立」升级为「发布结论不成立」，G-02/G-03 可升级为 P0，见 §7.3） |
| **P1** | **5** | G-01 G-02 G-03 G-04 G-05 |
| **P2** | **9** | G-06 G-07 G-08 G-09 G-10 G-11 G-12 G-13 G-14 |
| **P3** | **6** | G-15 G-16 G-17 G-18 G-19 G-20 |
| 合计 | **20** | 按类别：陈旧/矛盾 6、发布物说明 2、门禁与 CI 2、数量口径 4、失效指引/索引 2、证据管理 3、目标判定缺失 1 |

### 2.4 未达成 / 不可验证清单（供 Lead 直接引用）

**如实标注的未完成项（不是缺陷，是「已达成：如实标注」）**

1. Gate 6 Full100：未完成、未 seal，维持「关闭非 PASS」——证据：4 个 `run-full-*` 目录均无 `run_manifest.json`（§6 A3）。
2. 记忆参数标定（`H_anchor=5`/`E=10` 为默认而非标定）——ARCHITECTURE.md:115、HANDOFF.md:71；代码默认值 `crates/domain/src/chronicle.rs:15,17`。
3. CoT 三臂 × 80 轮：只有 PLAN/PROMPT，无 RESULT（`docs/workstreams/COT-THREE-ARM-80TURN-EVIDENCE-*.md`）。
4. JSON 生产写路径删除：未做，且被排除在本轮范围外（RELEASE-STATUS.md:11、ARCHITECTURE.md:186、HANDOFF.md:136）。

**不可/未独立验证项**

1. Android APK 为 release keystore 签名（无本地 apksigner 证据；仅能证实 workflow/脚本存在，见 §6 D2）。
2. Gitea Actions 停用、run 34–36 取消、`has_actions` 关闭（服务端状态）。
3. v0.1.2 三产物的「下载实测」过程（哈希数值与 GitHub 存储 digest 完全一致，可核；但「下载并实测」这一动作本身未留日志）。

---

## 3. 发现清单

### P1

#### G-01｜RELEASE-STATUS「停止位置」陈旧且自相矛盾（HEAD 漂移 + 不存在的未提交修复）
- **位置**：`docs/RELEASE-STATUS.md:34`（辅证 `:30`、`:84`、`:85`；`docs/release-closure-2026-09-06.md:156`）
- **证据**：
  - 原文（:34）：「HEAD 为 ce6117d（插件修复 + 版本 0.1.2 + 闭合证据）……工作区另有未提交的 SHA256SUMS 同名覆盖修复（`.github/workflows/release.yml` 与 `README.md`，校验和文件改按平台命名）」。
  - 实际：`git log --oneline -3` → `ab894c6 / ce6117d / b7d90b1`；`git status --porcelain` 为空；`git show --stat ab894c6` → `.github/workflows/release.yml | 9 +++++----`、`README.md | 2 +-`、`docs/RELEASE-STATUS.md | 29 ++++…`、`docs/release-closure-2026-09-06.md | 30 ++++…`，提交时间 `2026-09-06 14:18:08 +0800`。
  - `git rev-parse 'v0.1.2^{commit}'` → `ce6117db38ab…`（tag 确实指向 ce6117d，因此「据此发布 v0.1.2」成立；但「HEAD 为 ce6117d」「修复未提交」两条同时为假）。
- **影响**：本域核心矛盾。读者据 :34 会认为 (a) 仓库 HEAD 是 ce6117d，(b) 校验和修复尚未入库；实际是该修复已入库但**未进入 tag v0.1.2**（因此 v0.1.2 发布物仍缺平台命名校验和，与 G-02 叠加）。门禁表第 1 行「源码和文档一致性｜本轮更新」因此不成立。
- **建议**：把 :34 重写为「HEAD=ab894c6（含 SHA256SUMS 平台命名修复）；tag v0.1.2 = ce6117d，修复供 v0.1.2 之后版本生效」。
- **置信度**：高（已核实）

#### G-02｜README 承诺的平台命名校验和不存在于 v0.1.2 发布物
- **位置**：`README.md:43`
- **证据**：
  - 原文：「每个版本附按平台分文件的校验和：`SHA256SUMS-windows.txt`（Windows 安装器）与 `SHA256SUMS-android.txt`（APK）。」
  - 外部核实（GitHub API `repos/shis23/story/releases/tags/v0.1.2`，HTTP 200）：资产为
    `app-arm64-release.apk` 24,944,036 B / `StoryForge_0.1.2_x64-setup.exe` 8,893,240 B / `StoryForge_0.1.2_x64_en-US.msi` 12,226,560 B / **`SHA256SUMS.txt` 196 B**（github-actions 上传）/ **`SHA256SUMS-android.txt` 88 B**（用户手工补传）。
  - 即 v0.1.2 **没有** `SHA256SUMS-windows.txt`；Release 正文（workflow 在 tag 树生成）仍写「每个产物都有 `SHA256SUMS.txt` 校验」。
  - `docs/RELEASE-STATUS.md:82` 自己承认「workflow 已改为按平台命名……**供未来版本生效**」——与 README:43 的「每个版本附」直接冲突。
- **影响**：公开下载页给出的校验文件名与当前版本不符，用户按 README 校验会找不到文件；属对外发布说明不成立。
- **建议**：README 改为「自 v0.1.2 之后版本起按平台命名；v0.1.2 为 `SHA256SUMS.txt`（Windows 两产物）+ `SHA256SUMS-android.txt`（APK）」，并修正 Release 正文模板。
- **置信度**：高（已核实，外部 API）

#### G-03｜Android 下载通配符与实际资产名不匹配
- **位置**：`README.md:41`（同一字符串亦出现在 `.github/workflows/release.yml:247` 生成的 Release 正文）
- **证据**：README:41「**Android**：`*-arm64-*-release.apk`」；GitHub API 资产名为 `app-arm64-release.apk`。通配 `*-arm64-*-release.apk` 要求 `-arm64-` 之后还要出现一次 `-release.apk`，实际名字只有一个连字符（`arm64-release.apk`），**匹配失败**（PowerShell `Get-ChildItem -Filter` / bash glob 均不命中）。workflow 采集侧用的是 `*arm64*-release.apk`（能命中），因此是文档/正文模板写错。
- **影响**：用户按发布说明在 Releases 找不到对应文件。
- **建议**：统一改为 `app-arm64-release.apk` 或 `*arm64-release.apk`，并改 workflow 的 Release 正文模板。
- **置信度**：高（已核实，外部 API + 名字逐字符比对）

#### G-04｜README 首页仍称「真实模型移动写作 / 第三方插件 / 正式分发安装包 / 远端 CI 仍须闭合」
- **位置**：`README.md:25`
- **证据**：
  - 原文：「本轮 11 步确定性门禁、Windows 原生 IPC、Android 凭据生命周期、世界书往返和保留数据升级已通过。**真实模型移动写作、第三方插件、正式分发安装包和远端 CI 仍须分别闭合**，不能由调试包或浏览器测试替代。」
  - 对照 `docs/RELEASE-STATUS.md:29`：「当前候选完整真机写作、第三方插件｜**已闭合（本轮通过）**……Android 真机 9/9 环节……第三方插件两条通道」；`:28`：「正式分发安装包｜**已闭合（v0.1.2）**」；`:30`：「当前提交的远端 CI｜**已闭合**」；`:32` 总述四项「均已闭合」。
  - `git show ab894c6 -- README.md` 显示该提交只改了 README 的校验和那一行，第 25 行自 09-05 后未回写。
- **影响**：公开首页把已闭合的四项说成未闭合（与 RELEASE-STATUS 结论相反）；发布首页是外部读者第一入口。
- **建议**：按 RELEASE-STATUS:28–32 回写 README:25，保留「仍未闭合项」为：Windows 自身代码签名、任意历史回滚、ST 99 事件全集、JSON 写路径删除、记忆参数标定、CoT 长程。
- **置信度**：高（已核实）

#### G-05｜「Windows runner / Android 真机 / release 签名 / 第三方插件」四份文档状态互相冲突
- **位置**：
  - `docs/ARCHITECTURE.md:174`（「Linux Gitea runner 已投入运行；**Windows runner 执行尚未验证**」）、`:180-181`（「**release APK 签名无证书 BLOCKED**；**Android 真机与第三方插件现场矩阵仍缺**」）、`:187`（技术债 3「Windows runner、Android 真机、release 签名与可离线验证的真实产物证据」）
  - `docs/HANDOFF.md:16`（只到 v0.1.1）、`:30-31`（「Windows runner、桌面真实 GUI、Android 真机、签名安装包和第三方插件 iframe **仍缺现场证据**（release APK 签名无证书 BLOCKED）」）、`:79`、`:137`（下一优先级第 4 项同内容）
  - 对照 `docs/RELEASE-STATUS.md:28-30`、`:32`（全部「已闭合」；Android 真机 9/9、插件两通道、APK 为 release keystore 签名）
- **证据**：`artifacts/realmodel-2026-09-06/android/summary.md:5`（真机 9 环节 PASS）、`artifacts/plugin-acceptance-2026-09-06/summary.md:17-18`+`rerun-1b/summary.md`（插件两通道）、GitHub API run 34013739416（success）。
- **影响**：`ARCHITECTURE.md` 在 `CLAUDE.md` 必读清单第 3 位、`HANDOFF.md` 是交接入口，两者会让接手者把已闭合项当成待办，或反过来怀疑 RELEASE-STATUS 的闭合结论。唯一确有残余争议的是 **Gitea 自托管 Windows runner**（随 Gitea Actions 停用而作废），但三份文档没有做这一区分。
- **建议**：在三处各加一行「Gitea 自托管 Windows runner 已随 2026-09-06 Gitea Actions 停用作废；GitHub windows-latest 构建已由 run 34013739416 证实」，并把真机/签名/插件改为已闭合并给出证据路径。
- **置信度**：高（已核实）

### P2

#### G-06｜远端 CI 只构建、不跑 11 步确定性门禁；Pester「工作流合同」认证对象已被停用
- **位置**：`.github/workflows/release.yml:39-253`（三个 job 全文）；`docs/RELEASE-STATUS.md:21,30`；`scripts/tests/ReleaseBuild.RunnerReadiness.Tests.ps1:1328-1440`
- **证据**：
  - `release.yml` 两步 job 的全部命令为 `npm ci`、`npm run build`、`cargo install tauri-cli`、`cargo tauri build --ci`、产物收集与上传、`softprops/action-gh-release`；**无** `cargo fmt/clippy/test`、无 Pester、无 `verify-release.ps1`（全文 253 行已读）。
  - 门禁表 `:21` 行「发布脚本和工作流合同｜本轮通过｜四套 Pester 共 198 项……11 步统一入口已通过」——该「工作流合同」由本地 Pester 认证，其断言大量读取 `.gitea/workflows/ci-gates.yml`、`windows-gates.yml`、`release-host-evidence.yml`（测试文件 `:1328`、`:1340`、`:1348`、`:1421`、`:2464`、`:3069` 等）；而 `RELEASE-STATUS.md:30` 声明 Gitea Actions 已停用。
- **影响**：远端 CI 的「全绿」只等于「能构建出安装包」，不等于 11 步门禁在 CI 上执行过；且 198 项中的工作流合同有相当比例在认证一套已停用的 CI 配置（属覆盖范围失效，不是计数造假）。任何把「远端 CI 全绿」表述为「门禁已在 CI 验收」的结论都是过度引申。
- **建议**：Gate 表第 5 行加注「合同认证对象含已停用的 `.gitea/workflows/*`」；若要 CI 承载门禁，需新增 job 或在 README/RELEASE-STATUS 明确「11 步门禁仅在本地执行」。
- **置信度**：高（已核实；「认证对象已停用」的服务器侧状态无法离线核实，但文件存在与停用声明均为事实）

#### G-07｜发布工作流无 tag↔版本一致性校验
- **位置**：`.github/workflows/release.yml`（全文，无版本比对）；对照 `Cargo.toml:23`、`crates/tauri-app/tauri.conf.json:4`、`frontend/package.json:3` 均为 `0.1.2`
- **证据**：workflow 仅在 `push tags: ['v*']` 时构建并发布，未读取清单版本、未校验 tag 与 `tauri.conf.json`/`Cargo.toml` 一致；Gitea 侧的证据包校验反而有身份一致性断言（`ReleaseBuild.RunnerReadiness.Tests.ps1:693` `fails closed when manifest and provenance commit/branch/target disagree`）。
- **影响**：tag 与清单版本错配时仍会构建出「版本号与 tag 不符」的安装包并自动发布，发布完整性依赖人工纪律。
- **建议**：在 windows job 加一步比对 `github.ref_name` 与 `tauri.conf.json`/`Cargo.toml` 版本，不等则 fail。
- **置信度**：高（代码事实）

#### G-08｜DOCS-CODE-AUDIT「全部 175 个命令分布在 commands/*.rs」与代码不符
- **位置**：`docs/DOCS-CODE-AUDIT.md:11`（并见 `:28`、`:54`）
- **证据**：
  - `Select-String -Path (Get-ChildItem crates/tauri-app/src -Filter *.rs -Recurse).FullName -Pattern '#\[tauri::command' -AllMatches | Group-Object Path` → `commands/*.rs` 合计 **156**，`crates/tauri-app/src/card_studio_api.rs` **19**，总计 **175**。
  - `lib.rs:1301-1319` 以 `card_studio_api::cardstudio_*` 形式注册 19 个命令。
- **影响**：数量 175 正确（README:30 无误），但「全部分布在 `commands/*.rs`」的定位错误，会误导按目录检索命令的人；`DOCS-CODE-AUDIT.md` 被 `CLAUDE.md` 指定为「区分代码事实与计划的权威」。
- **建议**：改为「commands/ 156 个 + `card_studio_api.rs` 19 个 = 175 个，全部注册于 `lib.rs` 的 `generate_handler!`」。
- **置信度**：高（已核实）

#### G-09｜ST-EVENTS-COVERAGE 的事件常量数量与行号双双漂移
- **位置**：`docs/ST-EVENTS-COVERAGE.md:11`、`:48`
- **证据**：
  - 文档 :11「**32 个**事件类型常量在 `frontend/src/plugin-bridge.js:20-51`」；实际 `ST_EVENT_TYPES = Object.freeze({` 起于 `frontend/src/plugin-bridge.js:29`，止于 `:60`，成员 **30** 个（逐名枚举：APP_READY…EXTENSIONS_FIRST_LOAD）。
  - 文档 :48「`ST_EVENT_ALIASES`……定义在 `frontend/src/plugin-bridge.js:53-59`」；实际在 `:62-68`。
  - 文档自身表格只列 30 行事件，与「32」自相矛盾。
- **影响**：本文件是「ST 99 事件全集未承诺」的支撑材料，数量失真会削弱其可信度（结论方向仍是保守的，不构成夸大）。文件日期 `2026-07-08`，未随 card-shell/插件事件工作更新。
- **建议**：把 30/29-60/62-68 更正，并加「截至 2026-09-13 复核」时点注记。
- **置信度**：高（已核实）

#### G-10｜ROADMAP Phase 6 状态行指向不存在的 §11.3 章节
- **位置**：`docs/ROADMAP.md:149`
- **证据**：原文「Android 模拟器 15 项现场验收 PASS 见 `docs/HANDOFF.md` §11.3」；`Select-String -Path docs/HANDOFF.md -Pattern '11\.3'` 无命中（HANDOFF 全文 147 行，章节只有「当前结论/已落地主线/…/交接约束」，无编号 §11.3）；真正的 §11.3 在 `docs/workstreams/BACKEND-ARCHITECTURE-SQLITE-CLOSURE-RESULT-2026-07-28.md:373`（`### 11.3 验证证据`），且 HANDOFF.md:74 的标题「平台现场证据（Gate 6 §11.3 + Gate 7）」本身指向的就是该 RESULT。
- **影响**：接手者按 ROADMAP 去 HANDOFF 找 §11.3 会落空。
- **建议**：改为 `docs/workstreams/BACKEND-ARCHITECTURE-SQLITE-CLOSURE-RESULT-2026-07-28.md` §11.3。
- **置信度**：高（已核实）

#### G-11｜存在第三次完整门禁运行，文档只记两轮
- **位置**：`artifacts/release-gate-2026-09-06-final/gate-run.log`（目录 2026/9/6 14:19:57，日志 14:26:26，258,718 B）vs `docs/RELEASE-STATUS.md:34`、`:85`、`docs/release-closure-2026-09-06.md:156`
- **证据**：
  - `Get-ChildItem artifacts/release-gate-*` 得到三个目录：`release-gate-2026-09-06`（13:02:53）、`release-gate-2026-09-06-fixsums`（14:07:10）、**`release-gate-2026-09-06-final`（14:26:26）**；文档只列前两个。
  - `ab894c6` 提交时间 14:18:08，`release-gate-2026-09-06-final` 运行于其后 → 它才是**已提交 HEAD 树**的门禁证据，而文档指定的「第二轮」`-fixsums` 跑的是提交前工作树。
  - 三轮计数完全一致（我用同一正则对三份日志求和）：`suites=98 passed=1980 failed=0 ignored=33`。
- **影响**：证据索引与事实不符（P2）；同时说明「两轮门禁」的说法遗漏了唯一对齐 HEAD 的那一轮。三轮计数一致这一更强结论反而未被文档利用。
- **建议**：把 `-final` 日志补入 RELEASE-STATUS:34/:85 与 release-closure:156，并注明它是 HEAD `ab894c6` 的门禁证据。
- **置信度**：高（已核实）

#### G-12｜校验和「同名覆盖」缺陷方向疑似写反
- **位置**：`docs/RELEASE-STATUS.md:82`、`docs/release-closure-2026-09-06.md:144`
- **证据（疑似级）**：
  - 文档称「windows/android 两个 job 的校验和文件同名 `SHA256SUMS.txt`，上传时 **android 覆盖 windows**（v0.1.1/v0.1.2 均如此）」。
  - 外部核实（`releases/383478734/assets` + `releases/expanded_assets/v0.1.2`）：`SHA256SUMS.txt` = **196 B**，`digest=sha256:a23aac1d…`，`created_at=2026-09-06T05:42:42Z`（workflow 随 Release 一起上传，与 EXE/MSI/APK 同一时刻）；`SHA256SUMS-android.txt` = **88 B**，`digest=sha256:3a802507…`，`created_at=2026-09-06T05:58:05Z`，uploader = 用户 `shis23`（发布后 15 分钟手工补传，与 RELEASE-STATUS:82「已补传」吻合）。
  - 单条 `sha256sum`（64 hex + 2 空格 + 文件名 + 换行）对 APK 约 86–88 B ⇔ `SHA256SUMS-android.txt` 恰好一条；对两个 Windows 产物约 190–200 B ⇔ `SHA256SUMS.txt` 恰为两条。即 Release 上留下的 `SHA256SUMS.txt` 是 **Windows 的两行哈希**，被覆盖的是 **Android**，与文档所述方向相反。
  - 交叉印证：若真如文档所述「android 覆盖 windows」，Release 上会缺 Windows 哈希，补救动作应是补传 Windows 校验和；实际补救的是 Android 校验和。
  - 受限：未能取回 `SHA256SUMS.txt` 正文（GitHub 直链对本次工具返回非文本类型/超时），方向判定基于字节数+时间戳+补救动作三重推断，故整条仍标「疑似」。
- **影响**：根因方向写反，会误导后续排查（同名覆盖的先后顺序）；结论「缺陷已由平台命名修复」不受影响。
- **建议**：复核 Release 资产原文；如确认为 Windows 覆盖 Android，改正两处描述并注明覆盖方向由 job 完成顺序决定。
- **置信度**：中高（疑似，基于字节数推断 + 文档自身逻辑）

#### G-13｜插件验收证据清单不全
- **位置**：`docs/RELEASE-STATUS.md:91`
- **证据**：文档写「`artifacts/plugin-acceptance-2026-09-06/`（summary.md + rerun-1a/ + rerun-1b/）」；实际同名目录下还有 **`channel-a/`、`channel-b/`、`windows/`**（`Get-ChildItem … -Directory`）。其中 `channel-a/`、`channel-b/` 是首轮证据（缺陷证伪现场），`rerun-1*` 是复验证据。
- **影响**：`RELEASE-STATUS.md:52-53` 的核心叙述（初轮证伪 + 复验通过）依赖这两组目录，清单缺失使读者无法重建初轮现场。
- **建议**：补全为 `summary.md + channel-a/ + channel-b/ + windows/ + rerun-1a/ + rerun-1b/`。
- **置信度**：高（已核实）

#### G-14｜ROADMAP Phase 7 至今无状态标记与判定
- **位置**：`docs/ROADMAP.md:169-188`
- **证据**：Phase 1–6、8 均有 `**状态：…**` 行（:7、:33、:66、:91、:114、:149、:192），**Phase 7（:169）没有**；6 项任务（:175-180）无 `~~…~~ ✅` 勾选，3 条验收（:184-186）无判定，仅一句「详细执行计划文档已不随仓库分发」。
- **影响**：任务简报「Phase 1–8 全部标已完成」的前提不准确（Phase 6 = 主体完成、Phase 7 = 无状态）。Phase 7 的验收多数可由 `RELEASE-CHECKLIST.md`/`USER-GUIDE.md`/`PLAN-POST-MAINLINE.md` 间接覆盖，但「检查长会话性能、Agent 调用成本和移动端稳定性」（:178）在本仓库没有专门证据。
- **建议**：给 Phase 7 补状态行与逐项判定；对无证据项（长会话性能/成本）标「无专门证据/无法判定」。
- **置信度**：高（已核实）

### P3

#### G-15｜RELEASE-STATUS 把两轮复验的指标合并成一句
- **位置**：`docs/RELEASE-STATUS.md:56`
- **证据**：原文「prompt hook 标记 `[plugin-ok]` 进入生成轮**全部 4 类上游请求**（主生成/质量修订/摘要/后处理），**hookCalls=changes=15**」。
  - 「4 类请求」来自 `rerun-1a/`（第 3 轮 4 连调 + `r3-proxy-analysis.json`，`8/8 plugin_ok`）；
  - 「hookCalls=15 · changes=15」「events: 20」「0 DataCloneError」来自 `rerun-1b/summary.md:16,36`。
  - 两个数字各自可查（已核实），但同句呈现会被读成同一轮结果。
- **建议**：标注轮次归属。
- **置信度**：高（已核实）

#### G-16｜ROADMAP Phase 8 验收数字为 2026-07-08 时点且无注记
- **位置**：`docs/ROADMAP.md:210-211`
- **证据**：原文「双轨测试：node --test **212** pass + vitest **21** pass」「构建产物 **422KB**」；当前为 node 504 + vitest 119（门禁日志）与 `dist/assets/index-*.js` 603.22 kB。邻近的 `REGRESSION-COVERAGE.md:4` 对行号漂移加了时点注记，ROADMAP 此处没有（`:220-222` 只把读者指向 RELEASE-STATUS 作为「当前状态」）。
- **建议**：加「（2026-07-08 时点；当前计数见 RELEASE-STATUS）」。
- **置信度**：高（已核实）

#### G-17｜门禁数字无法用静态计数复现（口径说明）
- **位置**：`docs/RELEASE-STATUS.md:19`（33 忽略）与 §5 口径列
- **证据**：`Select-String -Pattern '#\[(tokio::)?test\]'` 得 **2009** 处属性，`#\[ignore` 得 **52** 处；门禁日志为 `passed=1980 + ignored=33 = 2013`。差异来自被 cfg/feature 排除、宏展开生成的用例与不计入 `#[test]` 的 doctest。
- **影响**：不是缺陷，但要求后续复核以**门禁日志**为口径，不要用静态计数判对错（本报告即按此口径）。
- **置信度**：高（已核实）

#### G-18｜REGRESSION-COVERAGE 行号已漂移（有注记但与列说明冲突）
- **位置**：`docs/REGRESSION-COVERAGE.md:12` 与 `:14`
- **证据**：`:12` 称「行号为文件当前位置」，`:14` 记录 `crates/domain/src/campaign.rs:389` 为 `resolved_persona_override_takes_priority`；实际该测试在 `crates/domain/src/campaign.rs:719`（`resolved_persona` 本身在 :384）。文档 `:4` 另有「行号会随代码漂移」注记，因此前后说明互相矛盾。
- **建议**：删除 `:12` 的「当前位置」措辞，或改为「以测试函数名检索为准」。
- **置信度**：高（已核实）

#### G-19｜RELEASE-CHECKLIST 历史段与当前状态并存且未标注为快照
- **位置**：`docs/RELEASE-CHECKLIST.md:59-66` vs `:199`
- **证据**：`:59` 标题为「2026-07-14 已验证（代码验证基线 `99b1ea3`）」，其中 `:63` 写「frontend `npm.cmd test` **311/311**」、`:66` 写「SQLite opt-in……（**默认仍 JSON**）」；而同一文件 `:199` 写「**存储默认已切换为 SQLite（Gate 7，2026-08-05，RESULT §36）**」。文件头 `:5` 只写「状态：2026-08-05 文档同步」，未把 07-14 段标为历史快照。
- **建议**：给该段加「历史快照（2026-07-14 时点，非当前状态）」标题。
- **置信度**：高（已核实）

#### G-20｜ARCHITECTURE-AUDIT（CLAUDE.md 必读第 3 位）对 MVU 闭环的结论已过期
- **位置**：`docs/ARCHITECTURE-AUDIT.md:3`（`> 状态：2026-06-17 更新版`）、`:139-141`、`:152`
- **证据**：
  - 原文 :141「`infra-plugin-host` 的 registry 已有，**`mvu_runtime.rs` 仍是 `StubMvuRuntime`**。`app-meta` 能分析 MVU，但分析结果还没有稳定进入 Campaign variable schema 和 UI 状态栏运行。」
  - 现状：`StubMvuRuntime` 仍存在于 `crates/infra-plugin-host/src/mvu_runtime.rs:98`（已降级为「桩，全部返回 NotImplemented（降级用）」，`trait` 说明见 `:61`），但生产路径已有 `crates/tauri-app/src/mvu_webview_runtime.rs`（WebView runtime），且 ROADMAP.md:126 记录「W10 已实现：DI 注入 + postprocess 调 `execute_fragment`」；变量 schema / 状态栏侧已有 `meta_apply_mvu_schema` 与 `frontend/src/components-v2/st/MvuStatusPanel.vue` + `utils/mvuKey.js`（CLAUDE.md「Current Code Facts」同口径）。
  - :152 的「MVU 部分已实现，插件权限分层部分未覆盖」中的后半句仍成立，但整段缺少时点注记；同文件其它条目（:63/:64/:67/:70/:73/:76）已用「阶段 N 已修复/已完成」标注，仅第 5 节停留在 06-17 判断。
- **影响**：该文件在 `CLAUDE.md` 必读清单第 3 位，接手者会据此认为 MVU 闭环未开始。属陈旧而非夸大。
- **建议**：给第 5 节加「2026-06-17 时点判断；MVU runtime 已于 W10 接通（ROADMAP:126），插件权限分层仍未覆盖」。
- **置信度**：高（已核实）

---

## 4. 逐条目标核对表

判定口径：达成 / 部分达成 / 未达成 / 无法判定。

### 4.1 ROADMAP Phase 1–8

| 声明出处 | 声明内容 | 代码/证据 | 判定 |
| --- | --- | --- | --- |
| ROADMAP.md:7-20 Phase 1 | 已完成（instance/definition fallback、CampaignRuntimeContext、Director instance、Plan instance_id、Subagent 输入、postprocess 名字→ID、删除角色清理） | `crates/domain/src/campaign.rs:384,394`（`resolved_persona`/`resolved_behavior`）+ 6 个单测 `:719-788`；`crates/domain/src/campaign_runtime.rs` 存在 | 达成 |
| ROADMAP.md:33-44 Phase 2 | 已完成（知识隔离、变量/任务注入、provenance、一致性测试） | `crates/app-agent/src/runtime.rs:685,730,735,869`（`build_campaign_subagent_volatile`、`current_character_instance_id`）；`BroadcastTarget`/`PropagationPolicy`（`crates/domain/src/character_knowledge.rs:34,182` + `app-agent/src/postprocess.rs:291-293`） | 达成 |
| ROADMAP.md:66-78 Phase 3 | 已完成（health check、explain、typed patch 8 action、preview/accept/dismiss、MVU apply、tool_center、6 个 campaign-aware 工具） | `crates/app-meta/src/{health_check,explain,typed_patch,mvu_apply}.rs` + `crates/app-agent/src/tool_center.rs` 均存在；命令 `meta_health_check`/`meta_explain_generation`/`meta_preview_typed_patch`/`meta_accept_typed_patch`/`meta_dismiss_typed_patch`/`meta_preview_mvu_apply`/`meta_apply_mvu_schema`（`commands/meta.rs:438,459`、`commands/meta_typed.rs:206,280,680,872,892`） | 达成 |
| ROADMAP.md:91-108 Phase 4 | 已完成（6 阶段：首屏聚焦/写作入口绑定/Campaign 4 tab/PipelinePanel/MetaPanel/移动端） | `frontend/src/stores/writing.js:49-52`（`writingMode` 三态 campaign/legacy/none）；`frontend/src/components-v2/{campaign,meta,ui,writing}` 目录；`frontend/src/AppV2.vue` | 达成 |
| ROADMAP.md:114-135 Phase 5 | 已完成（ST V2/V3 保真、raw JSON、fallback、bundle、PNG+lorebook、MVU apply 前端、WebView JS runtime） | `crates/tauri-app/src/mvu_webview_runtime.rs`；`commands/meta_typed.rs` MVU 命令；已知限制在 `:137-145` 如实列出（JS 变量归口、shim 覆盖度、Slash 冷门语义等） | 达成 |
| ROADMAP.md:149 Phase 6 | **主体完成**（Slice1 合并；模拟器 15 项 PASS；剩余真机回归） | `docs/workstreams/ANDROID-PHASE6-SLICE1-RESULT-2026-07-27.md` 存在；真机 9/9 于 09-06 补齐（`artifacts/realmodel-2026-09-06/android/summary.md`） | 部分达成（状态行指引错误 → G-10） |
| ROADMAP.md:169-188 Phase 7 | **无状态标记**（6 项任务 + 3 条验收未判定） | 无 ROADMAP 级判定；替代证据：`RELEASE-CHECKLIST.md`、`USER-GUIDE.md`、`PLAN-POST-MAINLINE.md` 存在；`:178`「长会话性能/Agent 调用成本」无专门证据 | 无法判定（→ G-14） |
| ROADMAP.md:192-212 Phase 8 | 已完成（9 阶段、8 commit 栈 `81e1608`…`ab70021`、双轨测试、422KB、契约红线） | `git cat-file -t 81e1608`/`ab70021` 均为 commit；`frontend/src/App.vue` 与旧 `components/*` 已删除（`Test-Path` false）、`components-v2/` 与 `AppV2.vue` 存在、`utils/*.js` 契约文件存在 | 达成（数字陈旧 → G-16） |

### 4.2 README「当前状态」8 条（README.md:18-25）

| 行 | 声明 | 核对证据 | 判定 |
| --- | --- | --- | --- |
| :18 | 版本 `0.1.2`，验收以 RELEASE-STATUS 为准 | `Cargo.toml:23`、`tauri.conf.json:4`、`package.json:3` 均 0.1.2；tag v0.1.2 → ce6117d；GitHub Release 非 draft 已发布 | 达成 |
| :19 | 三种生成模式 + 旧 `big_scene` 后端兼容 | `crates/domain/src/generation.rs:8-12`（`Continuation/Duet/BigScene/SequentialCrew`）；前端 `writing.js:49-52` | 达成 |
| :20 | 多角色 Campaign、重 roll、QualityGate、1× Editor auto-fix、私密归属门禁、Editor redaction | `crates/app-pipeline/src/draft_revision.rs:13`（`revise_draft`）；`crates/app-pipeline/src/lib.rs:2724,3687-3745`（`redact_performances_for_editor`）；`commands/writing_regenerate.rs` 存在；13 项 IPC 中 duet/sequential 两项均含「质量修订」 | 达成 |
| :21 | v3 Bundle 边界、已采纳历史保护、仅末尾分支 | 13 项 IPC 检查 1/4/5/6/7/8/9/10（`artifacts/realmodel-2026-09-06/windows/summary.md:27-36`）；ARCHITECTURE.md:93-99 边界一致 | 达成 |
| :22 | Chronicle M0–M4.2.2、ContextEpoch、A/B/C 工具、压缩 publication | `crates/domain/src/chronicle.rs`；`docs/AGENT_INTERFACES.md:62`（M0–M4.2.2 已落地）；`app-memory`/`app-pipeline` Chronicle 相关模块 | 达成 |
| :23 | M5 endurance runner 已合入；Gate 6 四项 PASS+seal、Full100 未完成、关闭非 PASS | `crates/harness-real-llm/src/endurance.rs`（91 KB）；4 份 sealed `run_manifest.json`（§6 A3）；4 个 `run-full-*` 无 manifest | 达成 |
| :24 | 默认 SQLite（Gate 7）、旧 JSON 自动迁移、JSON 显式回退 | `crates/tauri-app/src/lib.rs:1199-1224`（注释 + `resolve_backend`）；`storage_backend.rs:8,1928`；`docs/USER-GUIDE.md:194` | 达成 |
| :25 | 11 步门禁等已通过；**真实模型移动写作/第三方插件/正式分发安装包/远端 CI 仍须分别闭合** | 与 `RELEASE-STATUS.md:28/29/30/32` 四项「已闭合」直接冲突 | **未达成**（→ G-04） |

### 4.3 RELEASE-STATUS「当前门禁」14 行表（RELEASE-STATUS.md:17-30）

| # | 门禁 | 我的核对证据 | 判定 |
| --- | --- | --- | --- |
| 1 | 源码和文档一致性 | 与 :34 陈旧自述、README:43/:41/:25、ARCHITECTURE:174/180-181/187、DOCS-CODE-AUDIT:11 并存 | **未达成**（G-01/G-02/G-04/G-05/G-08） |
| 2 | 密钥扫描、fmt、严格 Clippy（含未跟踪输入扫描） | 门禁日志 `[1/11]`/`[3/11]`/`[4/11]` 全绿；未跟踪扫描实现见 `ReleaseBuild.Common.ps1:769-809`（`git ls-files --others --exclude-standard`，2 MiB 上限，取不到即 throw） | 达成 |
| 3 | Rust 1980 通过 / 0 失败 / 33 忽略，默认并发 | 三轮日志正则求和：98 suites、`passed=1980 failed=0 ignored=33`（每次一致） | 达成 |
| 4 | 前端 504+119+9+28+2=662；生产构建通过 | 三轮日志：node `ℹ tests 504 / pass 504 / fail 0`；vitest `Tests 119 passed`；csp `9 passed`；mobile-chrome `28 passed`；smoke `2 passed` | 达成 |
| 5 | 四套 Pester 198 项，无失败或跳过；11 步入口通过 | 日志 `Passed: 51/14/34/99`、`Skipped: 0`；静态 It 计数同为 198；`verify-release.ps1:46` `$TotalSteps=11`（覆盖范围问题见 G-06） | 达成 |
| 6 | Windows 系统凭据库一次性凭据实写/读/删 | `crates/infra-util/src/secret_store.rs:198` `system_keyring_write_read_delete_roundtrip`（`#[ignore]`，需显式 `--ignored`）；实跑记录 `release-closure-2026-09-06.md:37`（本轮未复跑） | 达成（间接） |
| 7 | Android 凭据生命周期真机通过 | `artifacts/realmodel-2026-09-06/android/summary.md:5`（9 环节）+ `artifacts/release-closure-2026-09-05/android/verification.json` 6 项含 Keystore 写/重启解密/删除/明文迁移 | 达成 |
| 8 | ST 世界书往返（两条世界书） | `android/verification.json` 第 6 项 `native ST export and reimport preserve current worldbook: passed`；`android-database-comparison.json` `campaign_world_info before=2 after=2 identical` | 部分达成（「两条」未在证据中显式计数；数值 2/2 一致可推断） |
| 9 | SQLite 并发与迁移；真机升级前后 20 张表一致 | `android-database-comparison.json`：20 个表条目全部 `identical: true`、`schema_migrations 8→8`、`integrity_before/after: ok`（与 release-closure:41 完全一致） | 达成 |
| 10 | Windows 原生写作（真实模型）13/13 + 33 次真实调用 | `realmodel-2026-09-06/windows/summary.md:7,23-39,43-58`：13 项逐条、33 次调用全 200（25 流式+8 非流式）、首 token 中位 2207 ms、单调用中位 70.9 s、token 下界 144,854（reasoning 85,916）；`ipc-verification.json` 76,815 B | 达成 |
| 11 | Windows/Android 调试候选产出并验证 | `release-closure-2026-09-06.md:39-41`（Redmi 23117RK66C、20 表对比）+ 上述 windows/android 证据目录 | 达成 |
| 12 | 正式分发安装包已闭合（v0.1.2） | GitHub API：run 34013739416 `conclusion=success`、`head_sha=ce6117d`；Release `draft=false`、`published_at=2026-09-06T05:42:43Z`；三产物 digest 与文档记载哈希逐个一致（APK `75bfc0c5…`、EXE `7759fece…`、MSI `df8bf8c2…`） | 达成（签名断言无法离线独立验证） |
| 13 | 真机写作 + 第三方插件已闭合 | `realmodel-2026-09-06/android/summary.md`（9 环节、SecretRef 无明文、正文 56→603 字）；`plugin-acceptance-2026-09-06/summary.md:17-18`（5/5 脚本 PASS、1A 两缺陷证伪）+ `rerun-1b/summary.md:16,36`（8/8 plugin_ok、hookCalls=changes=15、events=20、0 DataCloneError） | 达成 |
| 14 | 当前提交的远端 CI 已闭合 | run 34013739416（push tag，ce6117d）与 run 33980646148（workflow_dispatch，4948da7）均 success；Gitea 停用无法离线核实；不含 11 步门禁 → G-06 | 达成 |

### 4.4 ARCHITECTURE 技术债 4 项 与 HANDOFF 下一优先级 5 项

| 声明出处 | 内容 | 判定 | 依据 |
| --- | --- | --- | --- |
| ARCHITECTURE.md:185 | 100-turn 长程证据 + §35.8.5 两项探测缺口（非阻塞，另行立项） | 达成 | 如实标注；4 个 `run-full-*` 无 manifest |
| ARCHITECTURE.md:186 | 候选周期统计 + JSON 生产写路径删除（发布后） | 达成 | 如实标注；`docs/workstreams/BACKEND-ARCHITECTURE-SQLITE-CLOSURE-PLAN-2026-07-28.md:494` 同为「稳定期后另立计划」 |
| ARCHITECTURE.md:187 | Windows runner、Android 真机、release 签名、离线产物证据 | **部分达成（陈旧）** | 真机/签名已由 RELEASE-STATUS:28-29 闭合；Windows runner 随 Gitea 停用作废 → G-05 |
| ARCHITECTURE.md:188 | GUI 端到端、第三方插件现场矩阵 | **部分达成（陈旧）** | 插件现场已闭合（plugin-acceptance）；GUI 端到端部分由 13 项 IPC + 真机 9 环节覆盖 → G-05 |
| HANDOFF.md:134 | Full100 续跑已随 Gate 6 关闭移除 | 达成 | 如实标注 |
| HANDOFF.md:135 | Gate 8 文档封存收尾已完成（RESULT §37） | 达成 | `RESULT:2324-2414` §37 存在 |
| HANDOFF.md:136 | 发布后：候选周期统计；稳定期后删除 JSON 写路径 | 未达成（如实标注） | 与 ARCHITECTURE:186 一致 |
| HANDOFF.md:137 | Windows runner、Android 真机、签名包、真实第三方插件验收 | **部分达成（陈旧）** | 三项已闭合、Windows runner 作废 → G-05 |
| HANDOFF.md:138 | CoT 三臂 × 80 轮（PLAN §16 排期） | 未达成（如实标注） | `docs/workstreams/COT-THREE-ARM-80TURN-EVIDENCE-PLAN.md`/`-PROMPT.md` 存在，**无 RESULT**（`Get-ChildItem -Filter '*COT*RESULT*'` 空） |

### 4.5 未完成/受限项是否被如实标注（任务特别点名项）

| 项 | 声明 | 我的证据 | 判定 |
| --- | --- | --- | --- |
| Gate 6 关闭非 PASS | README:23、RELEASE-STATUS:32/:102、ARCHITECTURE:176-179、HANDOFF:22-24/:146、RELEASE-CHECKLIST:154-155 一致写「关闭 ≠ PASS」「不得写成 PASS」 | 措辞在 5 份文档中一致且带禁止性约束；`RESULT:1727,1860,2008-2009` 同口径 | **已核实，如实标注** |
| Full100 | 「未完成、未 seal、r3 58/100 全健康」 | `Get-ChildItem C:\Users\Predator\storyforge-evidence\gate6-2026-08-02\run-full-*` → 4 个目录，**均无 `run_manifest.json`**；4 个已 seal run 有 manifest | **已核实，如实标注（强证据）** |
| Windows runner | RELEASE-STATUS 不再提；ARCHITECTURE:174/HANDOFF:30,137 仍列为缺证据 | 两方均未说明「Gitea 自托管 Windows runner 已随停用作废」 | **未收敛（G-05）** |
| Android release 签名 | RELEASE-STATUS:28/:145「APK 为 release keystore 签名」 | 脚本 `scripts/ci-patch-android-signing.py`（key.properties 注入，失败即 return 1 → workflow 失败）；workflow 无 `apksigner verify` 步骤；无本地签名验证证据 | **疑似（无法独立验证）** |
| 记忆参数未标定 | ARCHITECTURE:115「H_anchor=5、E=10 是当前生产默认，**不得称为已标定参数**」、HANDOFF:71 | 代码 `crates/domain/src/chronicle.rs:15` `DEFAULT_H_ANCHOR=5`、`:17` `DEFAULT_E=10`（与「默认」一致）；无标定报告 | **已核实，如实标注** |
| CoT 三臂 | HANDOFF:32-33「计划在库中，RESULT 未写出」 | 仅 PLAN+PROMPT 两个文件，无 RESULT | **已核实，如实标注** |
| JSON 生产写路径删除 | RELEASE-STATUS:11 明确排除在范围外；ARCHITECTURE:186/HANDOFF:136 列为未来项 | 代码仍保留 JSON 后端与 `STORYFORGE_STORAGE_BACKEND=json`（`storage_backend.rs`）；无删除声明 | **已核实，如实标注** |
| ST 99 事件全集 | ROADMAP:145「ST 99 事件全集的真实后端 emit……仍待补」；ST-EVENTS-COVERAGE:5「**不是 ST 99 事件全集**」 | 该文件大量 ❌ 行；DOCS-CODE-AUDIT:22「不能承诺 ST 99 事件全集」 | **已核实，如实标注**（数量口径另见 G-09） |

---

## 5. 数量类事实核对表

| 事实 | 文档出处 | 我的统计命令 | 结果 | 口径可复现？ | 判定 |
| --- | --- | --- | --- | --- | --- |
| 16 个 crate | README:29、ARCHITECTURE:52、DOCS-CODE-AUDIT:28 | `(Get-ChildItem crates -Directory).Count`；`Select-String Cargo.toml 'crates/'` | 目录 16；workspace members 16（domain, infra-util, infra-import, infra-llm, infra-vector, infra-regex, infra-plugin-host, infra-sqlite, app-logging, app-agent, app-conversation, app-pipeline, app-memory, app-meta, tauri-app, harness-real-llm） | 是 | 达成 |
| 175 个 Tauri command | README:30、DOCS-CODE-AUDIT:11 | `Select-String -Pattern '#\[tauri::command' -AllMatches`（递归 `crates/tauri-app/src`）→ 分组计数；再解析 `lib.rs:1294-1487` 的 `generate_handler!` 块 | 属性 **175**（`commands/` 156 + `card_studio_api.rs` 19）；注册项 **175**（156 普通 + 19 `card_studio_api::*`）→ 定义与注册完全对齐 | 是 | 达成（**分布**声明错误 → G-08） |
| Rust 1980 通过 / 0 失败 / 33 忽略 | RELEASE-STATUS:19,34、README 三处 | 对三份 `gate-run.log` 正则 `test result: (ok\|FAILED)\. (\d+) passed; (\d+) failed; (\d+) ignored` 求和 | 三轮均 `suites=98, passed=1980, failed=0, ignored=33` | 是（以日志为准；静态属性计数 2009/52 不可复现 → G-17） | 达成 |
| 前端 504+119+9+28+2=662 | RELEASE-STATUS:20,34 | 从 `gate-run.log` 抽取：node `ℹ tests 504 / pass 504 / fail 0`；vitest `Tests 119 passed`；csp `9 passed`；mobile-chrome `28 passed`；smoke `2 passed`；另静态计数 `tests/*.test.mjs` 54 + stores 4 + composables 8 文件共 **504** 个 `test(`/`it(` | 662（=504+119+9+28+2） | 是（静态 504 亦可复现） | 达成 |
| Pester 四套 198 项、0 跳过 | RELEASE-STATUS:21,34 | 日志 `Passed: 51 / 14 / 34 / 99`、`Skipped: 0`；静态 `^\s*It\s` 计数 | 51+14+34+99 = **198**；静态同为 198 | 是 | 达成 |
| 11 步统一门禁 | README:57-63、RELEASE-CHECKLIST:36-48 | `verify-release.ps1:46` `$TotalSteps = 11`；脚本 `:155-182` 步骤清单；日志 `[1/11]…[11/11]` | 11 步：secret scan / Pester / fmt / clippy / cargo test / npm test / vitest / csp / mobile-chrome / smoke:ui / build | 是 | 达成 |
| 缺依赖/跳过不算通过 | README:59、RELEASE-CHECKLIST:50 | `run-release-build-tests.ps1:84-89`（无 Pester<5 直接 throw）；`ReleaseBuild.Tests.ps1:437-443`（TotalCount=0 / Skipped / Pending / Inconclusive 全 by throw）；`run-ui-smoke.ps1:34-43`（缺 Playwright `exit 2`，`verify-release.ps1:131-133` 对非 0 退出 throw） | 该声明成立（已核实） | 是 | 达成（「工作流合同」覆盖范围另见 G-06） |
| harness-real-llm「16 文件 ~15.7k 行」 | **任务简报**（非仓库文档，未在任何文档中找到该数字） | `Get-ChildItem crates/harness-real-llm -Recurse -File` → 42 文件；逐文件 `(Get-Content).Count` 求和 | 42 文件 / **34,597 行**（含空行；非空行 32,441），其中 39 个 `.rs` | 是 | 简报口径与现状不符（无文档声明，不计为发现） |
| v0.1.2 三产物哈希 | RELEASE-STATUS:75-80 | GitHub API `releases/tags/v0.1.2` 的 `digest` 字段 | APK `sha256:75bfc0c5…804a4`、EXE `sha256:7759fece…`、MSI `sha256:df8bf8c2…` 与文档记载值一致 | 是（数值层面；「下载实测」动作未留日志） | 达成 |
| 校验和文件 | README:43 | 同上 API 的 `assets[].name/size` | 实际 `SHA256SUMS.txt`(196 B)+`SHA256SUMS-android.txt`(88 B)，**无 `SHA256SUMS-windows.txt`** | 是 | **未达成**（G-02） |

---

## 6. 已核对一致项（无问题，供 Lead 免复查）

### A. 门禁与脚本

- A1 **11 步门禁结构正确**：`verify-release.ps1:170-182` 的 10 个 `Invoke-NativeStep` + 第 1 步 secret scan，全部 fail-fast（`:131-133` 非 0 退出即 throw）。
- A2 **Pester 缺依赖与跳过均 fail-closed**（详见 §5 对应行）；`Assert-ReleasePesterResult` 明确拒绝零执行。
- A3 **Gate 6 seal 证据在本机存在且与文档逐条吻合**：`C:\Users\Predator\storyforge-evidence\gate6-2026-08-02\` 下 4 个 run 目录各有 `run_manifest.json`，`status=completed`、`files=8`、sealed_at 分别为 **2026-08-02T14:17:30Z / 16:23:21Z / 16:38:31Z / 23:58:23Z**，与 `RESULT §35.2.1-4` 记载完全一致；模型 `deepseek-v4-flash`、commit `1c1fc4e/f0709337` 有记录。4 个 `run-full-*` 目录**无 manifest**，与「Full100 未 seal」一致。
- A4 **第三次门禁日志与文档两轮计数一致**：三轮 `1980/0/33` 与前端 662、Pester 198 完全相同（文档漏记该轮 → G-11，但数字本身无误）。
- A5 **secret scan 覆盖未跟踪构建输入**：`ReleaseBuild.Common.ps1:781` 调 `git ls-files --others --exclude-standard`，`:794/:809` 对异常路径直接 throw（不静默跳过）。
- A6 **Pester 版本约束与文档一致**：`run-release-build-tests.ps1:80-89` 只接受 Pester <5（执行日志为 Pester 3.4.0；README:59 推荐 4.10.1，属「推荐」不矛盾）。

### B. 目标与代码事实

- B1 ROADMAP Phase 1–5 的关键符号全部存在（见 §4.1 表；含 `resolved_persona/behavior`、`CampaignRuntimeContext`、`build_campaign_subagent_volatile`、`BroadcastTarget::{All,Group}`、`PropagationPolicy`、`health_check/explain/typed_patch/mvu_apply`、`tool_center.rs`、`writingMode` 三态）。
- B2 Phase 8 的两个边界 commit（`81e1608` 脚手架、`ab70021` 删除旧前端）均为真实 commit；旧 `App.vue` 与 `frontend/src/components/*` 已删除，`components-v2/` 与 `AppV2.vue` 存在。
- B3 默认 SQLite 与 README/USER-GUIDE/HANDOFF 一致（`lib.rs:1199-1224`、`storage_backend.rs:8`、`USER-GUIDE.md:194`）。
- B4 记忆参数默认值 `H_anchor=5`/`E=10` 在代码中确认（`chronicle.rs:15,17`），且文档明确「不得称为已标定」。
- B5 未完成四项（Full100 / 记忆标定 / CoT 三臂 / JSON 写路径删除）在 5 份以上文档中口径一致、无夸大。
- B6 ST 99 事件覆盖的**保守结论**成立：`ST-EVENTS-COVERAGE.md:5` 自我限定，`plugin-acceptance/summary.md:106` 同样把「ST 99 事件全集兼容」列为未纳入边界。
- B7 `docs/DATA_MODEL.md`（CLAUDE.md 必读第 5 位）关键声明与代码一致：`CharacterInstance` 字段分层、`context_epoch: ContextEpochSnapshot`（:19）、变量归属（:146-147）、`tool_whitelist` 的 `None`/`Some([])`/`Some(list)` 语义（:181）、内置默认 profile `builtin-default-agent-v1` 不可删除（:197）均与 CLAUDE.md「Current Code Facts」及代码同口径，未发现冲突。
- B8 `docs/ARCHITECTURE-AUDIT.md` 的阶段 2/5/6 结论（:63/:64/:67/:70/:73/:76）与代码一致（`fill_campaign_context`、`WritingContext/ToolContext.campaign_runtime`、`persist_postprocess_outcome` instance 解析、`delete_character` 级联、`pending_temporary_instances`）；仅第 5 节过期（→ G-20）。

### C. 外部可验证的发布与真机声明

- C1 GitHub Actions **run 34013739416**：`conclusion=success`、`event=push`、`head_sha=ce6117db…`、`head_branch=v0.1.2`（与 RELEASE-STATUS:30/:77 一致）。
- C2 GitHub Actions **run 33980646148**：success、`event=workflow_dispatch`、`head_sha=4948da71…`（与 :30 一致）。
- C3 GitHub Release v0.1.2：`draft=false`、`prerelease=false`、`published_at=2026-09-06T05:42:43Z`（与 :28 一致）；5 个资产（3 产物 + 2 校验和文件）。
- C4 Windows 真实模型 13/13 与全部指标（33 次调用、首 token 中位 2207 ms、单调用中位 70.9 s、token 下界 144,854/reasoning 85,916、tool_mode=native 一次通过、L1 仅 SecretRef）与 `artifacts/realmodel-2026-09-06/windows/summary.md` 逐项一致，且该摘要如实记录了驱动偏差（代理 bug、max_tokens 4096→8192、采纳轮询 60s→300s）。
- C5 Android 真机：9 个逻辑环节（summary.md:5 枚举）与明细表 12 行不矛盾；SecretRef 无明文、正文 56→603 字、冷重启持久、全程 GUI 建连接均有 JSON/截图证据。
- C6 插件通道 1B 5/5 脚本 PASS、1A 初轮证伪 + 复验通过；`hookCalls=changes=15`、`events=20`、`0 DataCloneError`、`8/8 plugin_ok` 均可在 `rerun-1a/`、`rerun-1b/` 找到（仅句内归属不清 → G-15）。
- C7 09-05 收尾证据齐全：`android/verification.json` 6 项全 pass（含世界书往返）、`android-database-comparison.json` 20 表全 identical + 8 条迁移账本 + `integrity ok`、`gate-verified.log` 254 KB。
- C8 **本域全部范围内文档的 Markdown 相对链接扫描无失效目标**（对 README、CLAUDE.md、RELEASE-STATUS、ROADMAP、ARCHITECTURE、HANDOFF、DOCS-CODE-AUDIT、RELEASE-CHECKLIST、USER-GUIDE、FRONTEND-COMPONENTS、REGRESSION-COVERAGE、ST-EVENTS-COVERAGE、release-closure、INTENT、TECHNICAL_DESIGN 逐一 `Test-Path` 相对链接）。
- C9 口径澄清（避免误报）：任务清单中的 `docs/AGENT-INTERFACES.md`（连字符）不存在，仓库实际文件为 `docs/AGENT_INTERFACES.md`（下划线），`README.md:100` 与 `CLAUDE.md` 指向的都是存在的下划线版本 —— **不是缺陷**。

### D. 需要标注但不构成文档缺陷的边界

- D1 Gitea Actions 停用、run 34–36 取消、`has_actions=false` 属服务端状态，本机无证据（记入「不可独立验证」）。
- D2 APK release keystore 签名：`scripts/ci-patch-android-signing.py` 只在 `key.properties` 存在时注入，`build.gradle.kts` 无签名时 Gradle 仍能产出（未签名）APK；workflow 未验证产物签名。RELEASE-STATUS:28 的签名断言**方向可信但未独立验证**，建议补 `apksigner verify` 证据。
- D3 `artifacts/release-closure-2026-09-05/python/**` 部分子目录当前读取被拒（`Access to the path … is denied`），`Get-ChildItem -Recurse` 会报错——属本机 ACL，不影响上述结论。

---

## 7. 需要 Lead 重点复核的结论

1. **G-01 是「发布状态」文档的核心矛盾**：`RELEASE-STATUS.md:34` 同时给出错误 HEAD（ce6117d 而非 ab894c6）与不存在的未提交修复；建议在最终报告中把它作为「文档一致性门禁不成立」的直接证据，并采用 `artifacts/release-gate-2026-09-06-final/gate-run.log`（14:26，对齐 HEAD）作为门禁证据。
2. **G-02 + G-03 是唯一会误导真实用户的对外声明**（校验和文件名、APK 下载通配符），且 Release 正文模板同样带错。两者都在 tag 树中，v0.1.2 之后版本若只改 workflow 不改模板仍会复现。
3. **P0 判定口径请 Lead 裁定**：本域判定 P0=0（核心验收无虚假）；但若评审把「对外发布说明与实际发布物不符」视为「发布结论不成立」，则 G-02/G-03 应升级为 P0。升级与否会改变整份汇总报告的最严重级别。
4. **跨文档状态收敛（G-05）需要一次统一回写**：ARCHITECTURE（必读清单）、HANDOFF（交接入口）、README（首页）三处与 RELEASE-STATUS 冲突，涉及 Windows runner / Android 真机 / release 签名 / 第三方插件四项；其中只有「Gitea 自托管 Windows runner」是真实残余（且已因停用而作废），其余三项确有证据。
5. **远端 CI 与门禁的关系不要在最终结论中被合并表述（G-06）**：GitHub tag 构建 success ≠ 11 步门禁在 CI 执行；198 项 Pester 里含有对已停用 `.gitea/workflows` 的合同断言。
6. **证据管理两条建议**：(a) 记录第三次门禁日志（G-11）；(b) 补全 plugin-acceptance 目录清单（G-13）；(c) APK 签名补 `apksigner verify` 证据（D2）。
7. **Phase 7 的「无法判定」需在总报告体现（G-14）**：任务简报称「Phase 1–8 全部标已完成」，实际 Phase 6 = 主体完成、Phase 7 无状态，且「长会话性能 / Agent 调用成本」无专门证据。
8. **数字漂移的根因是缺少脚本生成（G-08/G-09/G-16/G-17）**：`DOCS-CODE-AUDIT.md:28` 自己建议「改为脚本生成计数」，至今未做；建议把 16/175/1980/662/198 做成 CI 或脚本产物。

---

### 附：判定与置信度用语约定

- **已核实（高置信）**：有代码行号或命令输出直接支撑。
- **疑似（中/中高置信）**：有间接推断链但有一步未能直接取回原始对象（如 G-12 的 `SHA256SUMS.txt` 原文）。
- **无法判定**：缺少可核证据，且不能在本地/公开渠道补齐（如 Gitea 服务端状态、Phase 7 判定）。

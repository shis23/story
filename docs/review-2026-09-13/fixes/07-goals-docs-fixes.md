# 修复记录 07：文档回写 A（RELEASE-STATUS / README / ARCHITECTURE / HANDOFF）

- 任务：**task-14**（文档回写 A，G-01 / G-02 / G-03 / G-04 / G-05 / G-11 / G-06）
- 执行人：review-goals（域7 声明审计员）
- 日期：2026-09-13
- 基线：`main@ab894c6`（2026-09-06 14:18:08 +08）；执行时工作区另有修复阶段的代码改动（不属于本任务，未触碰）
- 写作用域（全部改动仅限）：`docs/RELEASE-STATUS.md`、`README.md`、`docs/ARCHITECTURE.md`、`docs/HANDOFF.md`、本文件
- 未改动：`.github/**`、`CLAUDE.md`、任何源码/脚本、`docs/review-2026-09-13/07-goals-and-claims.md`（审查产物）
- 未执行：任何 cargo / npm / `verify-release.ps1`（门禁由 Lead 统一执行）

> 记录格式：每条给出 **原声明 → 改后声明 → 依据（命令/API/代码行）**。

---

## 1. G-01｜RELEASE-STATUS「停止位置」HEAD 与未提交修复的自相矛盾

- **位置**：`docs/RELEASE-STATUS.md` 停止位置段（原第 34 行）
- **原声明**：「HEAD 为 ce6117d（插件修复 + 版本 0.1.2 + 闭合证据），已推送 Gitea+GitHub 双远端并据此发布 v0.1.2；工作区另有未提交的 SHA256SUMS 同名覆盖修复（`.github/workflows/release.yml` 与 `README.md`，校验和文件改按平台命名）」；且只记两轮门禁。
- **改后声明**（要点）：
  1. 仓库 HEAD 为 `ab894c6`（`fix(release): platform-suffixed checksum files; record v0.1.2 closure`，2026-09-06 14:18:08 +08），**已包含**SHA256SUMS 平台命名修复；
  2. 2026-09-13 复核时 `git status --porcelain` 为空；HEAD 已推送到 Gitea 远端（`origin/main` = ab894c6）；
  3. 指向 GitHub 的本地 remote-tracking 引用 `github/main` 停在 `3b114fe`（2026-09-02），**故不声称 ab894c6 已推送到 GitHub**；
  4. **发布用的 tag `v0.1.2` 指向 `ce6117d`**，平台命名校验和修复不在 v0.1.2 发布物内，自下一版本生效；
  5. 门禁改为三轮（见 G-11）。
- **依据**：
  - `git log -1 --format='%H %cI %s'` → `ab894c60a1d5f8b8ff403131f548425865623271 2026-09-06T14:18:08+08:00 fix(release): platform-suffixed checksum files; record v0.1.2 closure`
  - `git status --porcelain`（2026-09-13 复核当时）→ 空
  - `git show --stat ab894c6` → 触及 `.github/workflows/release.yml`、`README.md`、`docs/RELEASE-STATUS.md`、`docs/release-closure-2026-09-06.md`
  - `git rev-parse 'v0.1.2^{commit}'` → `ce6117db38ab80429644f42a134532a8c825cc6c`；`git cat-file -p <tag>` → tag 对象指向 ce6117d
  - `git rev-parse origin/main` → `ab894c6…`；`git rev-parse github/main` → `3b114feb…`；`git remote -v` → origin = `https://git.2529985.xyz/ss/story.git`（Gitea），github = `https://github.com/shis23/story.git`
  - 门禁表第 1 行同步改为「已回写（2026-09-13 复核）」，通过条件写明本次回写内容并链接本记录
- **影响面**：`README.md` 与 `docs/ARCHITECTURE.md` 的关联表述已随 G-02/G-04/G-05 一并回写，三份文档对 HEAD/tag 的口径现已一致。

## 2. G-02｜README 的平台校验和承诺与实际发布物不符

- **位置**：`README.md:43`
- **原声明**：「每个版本附按平台分文件的校验和：`SHA256SUMS-windows.txt`（Windows 安装器）与 `SHA256SUMS-android.txt`（APK）。」
- **改后声明**：「校验和文件：v0.1.2 的 Release 资产是 `SHA256SUMS.txt` 与 `SHA256SUMS-android.txt`（APK 哈希，发布后补传）；平台命名文件 `SHA256SUMS-windows.txt` / `SHA256SUMS-android.txt` 自 v0.1.2 之后的版本起生效，下载时以 Release 页面上的实际文件名为准。」
- **同步改动**：`docs/RELEASE-STATUS.md` 门禁表第 12 行补充实际资产事实（196 B / 88 B，**没有** `SHA256SUMS-windows.txt`，平台命名自下一版本生效）。
- **依据**（GitHub API，非文档自证）：
  - `GET /repos/shis23/story/releases/383478734/assets`：`SHA256SUMS.txt` size=196、`created_at=2026-09-06T05:42:42Z`、digest `sha256:a23aac1d…`；`SHA256SUMS-android.txt` size=88、`created_at=2026-09-06T05:58:05Z`、uploader=`shis23`、digest `sha256:3a802507…`
  - `GET /repos/shis23/story/releases/expanded_assets/v0.1.2`：仅上述 5 个资产（3 产物 + 2 校验和），无 `SHA256SUMS-windows.txt`
  - 该方向与 `RELEASE-STATUS.md` 原有「workflow 已改按平台命名，供未来版本生效」的表述一致
- **未采用方案**：在 workflow 中补生成 `SHA256SUMS-windows.txt`（`.github/**` 不在本任务写作用域）→ 见「需 Lead 决策项 1」。

## 3. G-03｜Android 下载通配符与实际资产名不匹配

- **位置**：`README.md:41`
- **原声明**：「**Android**：`*-arm64-*-release.apk`，……」
- **改后声明**：「**Android**：`app-arm64-release.apk`（匹配模式 `*arm64-release.apk`），允许「安装未知来源应用」后安装（仅支持 arm64 设备）。」
- **依据**：GitHub API 资产名为 `app-arm64-release.apk`（24,944,036 B）；原模式 `*-arm64-*-release.apk` 要求 `-arm64-` 之后再次出现 `-release.apk`，实际名只有单个连字符，匹配失败（workflow 采集侧用的是 `*arm64*-release.apk`，可命中）。
- **未改动**：`.github/workflows/release.yml` 生成的 Release 正文模板中同一字符串（`*-arm64-*-release.apk`）仍存在 → 见「需 Lead 决策项 2」。

## 4. G-04｜README「当前状态」与四项「已闭合」冲突

- **位置**：`README.md:25`
- **原声明**：「本轮 11 步确定性门禁、Windows 原生 IPC、Android 凭据生命周期、世界书往返和保留数据升级已通过。**真实模型移动写作、第三方插件、正式分发安装包和远端 CI 仍须分别闭合**，不能由调试包或浏览器测试替代。」
- **改后声明**：11 步门禁改为「三轮全绿且计数完全一致」并列出计数；四项改为「已闭合」并给出证据要点（Windows 原生 13/13、Android 真机 9/9、插件两通道、v0.1.2 三产物哈希、GitHub tag 构建全绿），链接 `docs/RELEASE-STATUS.md`；**仍未闭合项改为有证据的列表**：Windows EXE 代码签名、任意历史回滚、ST 99 事件全集、JSON 生产写路径删除、记忆参数标定、CoT 长程研究。
- **依据**：`RELEASE-STATUS.md:28`（正式分发安装包 已闭合 v0.1.2）、`:29`（真机写作 + 第三方插件 已闭合）、`:30`（远端 CI 已闭合）、`:32`（整体状态）；`artifacts/realmodel-2026-09-06/{windows,android}/summary.md`；`artifacts/plugin-acceptance-2026-09-06/summary.md`；GitHub run 34013739416（success, head_sha=ce6117d）。
- **口径保持**：仍保留「调试包或浏览器测试不能替代真机与真实产物验证」，与 RELEASE-STATUS「边界（不得夸大）」一致。

## 5. G-05｜ARCHITECTURE / HANDOFF 与 RELEASE-STATUS 的闭合状态冲突

原则：不篡改历史证据，改为**区分「已闭合」与「真实残余」并标注时点**。

### 5.1 `docs/ARCHITECTURE.md`

| 位置 | 原声明 | 改后声明 | 依据 |
| --- | --- | --- | --- |
| `:3` | 更新日期：2026-09-05 | 更新日期：2026-09-13（说明仅「插件与导入边界/发布边界/技术债」按 09-06 结果回写，其余仍为 09-05 快照） | 本次改动范围 |
| `:169` | 插件 iframe、真实第三方扩展和完整 ST 长尾语义**仍需 GUI 验收** | 插件 iframe 与真实第三方扩展样本**已由 2026-09-06 现场验收覆盖**（TavernHelper 远程脚本 5/5 + manifest 插件 prompt hook 复验）；完整 ST 长尾语义（99 事件全集、冷门 Slash/TavernHelper API）仍需补 | `plugin-acceptance-2026-09-06/summary.md:17-18`、`rerun-1b/summary.md` |
| `:174` | 「Windows runner 执行尚未验证」 | Gitea Actions 已于 2026-09-06 停用（act_runner/容器停止、排队 run 不再执行、has_actions 关闭）；远端构建改由 GitHub Actions 承担，v0.1.2 tag 构建（windows-latest/ubuntu-latest/release 三 job）由 run 34013739416 证实；**GitHub 侧只构建，不执行 11 步门禁** | RELEASE-STATUS:30/:84；GitHub API run 34013739416 |
| `:180-181` | 「release APK 签名无证书 BLOCKED；Android 真机与第三方插件现场矩阵仍缺」 | 段首行标注为 2026-08 阶段记录；新增一行：2026-09-06 补齐 Android 真机 9/9 与插件两通道；APK 为 release keystore 签名但**缺少可离线复现的 `apksigner verify` 证据**；Windows EXE 仍无代码签名 | `artifacts/realmodel-2026-09-06/android/summary.md:5`；RELEASE-STATUS:28/:29 |
| `:187` | 技术债 3：「Windows runner、Android 真机、release 签名与可离线验证的真实产物证据」 | 改为：Windows 自托管 runner 已随 Gitea 停用作废（改由 GitHub-hosted runner）；APK 签名缺少 `apksigner verify` 证据；离线可验证的真实产物证据**已由 v0.1.2 Release（三产物大小与 SHA-256）提供** | GitHub API 三资产 size+digest 与 RELEASE-STATUS:79-81 一致 |
| `:188` | 技术债 4：「GUI 端到端、第三方插件现场矩阵」 | 改为：2026-09-06 已覆盖 Windows 原生 13 项 IPC、Android 真机 9 环节与插件现场样本；完整矩阵与第三方脚本长尾仍待补 | `realmodel-2026-09-06/*/summary.md`；plugin-acceptance |

### 5.2 `docs/HANDOFF.md`（注意：该文件被 `.gitignore:92` 忽略，见「需 Lead 决策项 3」）

> **⚠ 本地文件声明（2026-09-13 增补，Lead 复核要求）**：`docs/HANDOFF.md` 是**本机未入库文件**——
> `git ls-files --error-unmatch docs/HANDOFF.md` 失败、`git check-ignore -v` 命中 `.gitignore:92`、
> `git status` 中也看不到它。因此本节的回写**只存在于本机工作区，不会进入任何 commit，也不会出现在
> 其它克隆或新克隆的仓库里**；请勿把本节当作已入库的文档变更。
> 相关问题更广：`docs/` 下有 27 个被 ignore 的文件，其中含 `CLAUDE.md`「Required Reading Order」第 1 项
> `docs/DOCS-CODE-AUDIT.md` 与第 3 项 `docs/ARCHITECTURE-AUDIT.md`（新克隆无法遵循项目自己的必读顺序）。
> 该问题由 Lead 的第二版报告与 **task-16** 处理；本轮按 Lead 决定**不动 `.gitignore`**。

| 位置 | 原声明 | 改后声明 | 依据 |
| --- | --- | --- | --- |
| `:3` | 更新日期：2026-09-01 | 保留 09-01，加括注「2026-09-13 增补 v0.1.2 发布与闭合状态；其余段落仍为 09-01 基线快照」 | 本次改动范围 |
| `:26` 后 | 无 v0.1.2 信息 | 新增「2026-09-13 增补」段落：tag `v0.1.2` → `ce6117d` 于 2026-09-06T05:42:43Z 正式发布（run 34013739416）；平台命名修复在其后的 HEAD `ab894c6` 上、不在 v0.1.2 内；入口以 RELEASE-STATUS 为准 | GitHub API release/run；git ref 事实 |
| `:30-31` | 「Windows runner、桌面真实 GUI、Android 真机、签名安装包和第三方插件 iframe 仍缺现场证据（release APK 签名无证书 BLOCKED）」 | 改为：Android 真机 9/9、插件两通道、签名安装包（v0.1.2）均已闭合；**真实残余**是 Gitea 自托管 runner 随停用作废 + APK 缺少 `apksigner` 证据；原「无证书 BLOCKED」保留为 2026-08 阶段记录 | 同 5.1 |
| `:46` | 发布证据行「……Windows runner 实跑尚未验证」 | 改为：08 基线 + Gitea 停用 + 改由 GitHub-hosted runner（run 34013739416，仅构建不含门禁） | 同 5.1 |
| `:79` | 「release APK 签名无证书 BLOCKED。」 | 「……当时无证书 BLOCKED（v0.1.2 已于 2026-09-06 改用 release keystore 签名发布）」 | RELEASE-STATUS:28/:83 |
| `:137` | 下一优先级 4「Windows runner、Android 真机、签名包（需证书）与真实第三方插件验收」 | 划为已完成（2026-09-06，v0.1.2），并保留 apksigner 证据待补 | 同 5.1 |

## 6. G-11｜补记第三次完整门禁运行

- **位置**：`docs/RELEASE-STATUS.md` 门禁表第 4 行、整体状态段、停止位置段、v0.1.2 闭合节、证据目录
- **原声明**：「两轮完整 11 步门禁均通过且计数完全一致」，只列 `artifacts/release-gate-2026-09-06/` 与 `-fixsums/` 两份日志。
- **改后声明**：改为**三轮**，列出三分日志并给出每轮性质与日志尾部事实：
  - 第一轮 2026-09-06 13:02（插件修复 + 版本 0.1.2 最终树）：日志含 `GATE_EXIT_CODE=0`；
  - 第二轮 14:07（SHA256SUMS workflow/README 修复后）：日志以 `Release gate passed.` 结束，**未写入退出码行**；
  - 第三轮 14:26（提交后 HEAD 树，对齐 `ab894c6`）：同上，未写入退出码行；
  - 三轮计数完全一致（1980 通过/0 失败/33 忽略、662、198）。
  - 证据目录补齐第三份日志；门禁表第 4 行注明「第三轮运行于已提交的 HEAD 树」。
- **依据**：
  - `Get-ChildItem artifacts/release-gate-2026-09-06*` → 三个目录，日志大小/时间 261,708 B @13:02:53 / 258,711 B @14:07:10 / 258,718 B @14:26:26；`ab894c6` 提交时间 14:18:08 早于第三轮
  - 三轮 `tail`：均以 `Release gate passed.` 结束；`Select-String GATE_EXIT_CODE` 计数分别为 1 / 0 / 0
  - 三轮正则求和 `test result: (ok|FAILED)\. (\d+) passed; (\d+) failed; (\d+) ignored` → 每轮 `suites=98, passed=1980, failed=0, ignored=33`；前端 `tests 504 / pass 504 / fail 0`、`Tests 119 passed`、csp 9、mobile 28、smoke 2；Pester `Passed: 51/14/34/99`、`Skipped: 0`
- **口径纪律**：**不写「三轮退出码均为 0」**——只有第一轮日志含退出码行，第二、三轮以终结行 `Release gate passed.` 为证（已在文档中显式说明）。

## 7. G-06｜远端 CI 只构建不跑门禁 / Pester 认证已停用的 `.gitea`

- **位置**：`docs/RELEASE-STATUS.md` 门禁表第 5 行与第 14 行；`docs/ARCHITECTURE.md:174`
- **原声明**：门禁表第 14 行「当前提交的远端 CI｜已闭合」未说明范围；第 5 行「发布脚本和工作流合同｜四套 Pester 共 198 项……11 步统一入口已通过」未说明合同对象。
- **改后声明**：
  - 第 14 行补「范围说明」：release.yml 三个 job 只做构建与产物上传（windows/android：`npm ci` + `npm run build` + `cargo tauri build --ci`；release：汇总上传），**不含 fmt/clippy/`cargo test`/Pester**——11 步确定性门禁只在本地 `scripts/verify-release.ps1` 执行；
  - 第 5 行补「范围说明」：其中「工作流合同」断言仍读取 `.gitea/workflows/*`（ci-gates / release-host-evidence / windows-gates 三份），而 Gitea Actions 已于 2026-09-06 停用——该组断言认证的是一套**已停用**的 CI 配置，不等于 GitHub Actions 侧存在门禁；
  - `ARCHITECTURE.md:174` 同口径加一句「GitHub 侧 job 只做构建与产物上传，不执行 11 步确定性门禁」。
- **依据**：
  - `.github/workflows/release.yml`：`jobs:` `:35`；`windows:` `:39`/`runs-on: windows-latest` `:40`；`npm ci` `:64`；`npm run build` `:65`；`run: cargo tauri build --ci` `:77`；`android:` `:108`/`:109`；`npm ci` `:149`；`npm run build` `:150`；`release:` `:219`/`:221`。全文无 `verify-release`、`cargo test`、`cargo clippy`、`cargo fmt`、Pester 调用。
  - `.gitea/workflows/` 三份文件仍在库中：`ci-gates.yml` 11,730 B、`release-host-evidence.yml` 8,194 B、`windows-gates.yml` 5,203 B。
  - Pester 引用证据：`scripts/tests/ReleaseBuild.CI.Tests.ps1` 5 处（行 466/484/493/503/697）；`scripts/tests/ReleaseBuild.RunnerReadiness.Tests.ps1` 47 处（自 1328 行起）。
- **未改动**：`.gitea/**`、`.github/**`、Pester 测试（超出写作用域）→ 见「需 Lead 决策项 4」。

## 8. 顺带修正（同一文件、同一证据来源，已在此显式登记）

| ID | 位置 | 原声明 | 改后声明 | 依据 |
| --- | --- | --- | --- | --- |
| G-12（疑似） | `docs/RELEASE-STATUS.md` 校验和缺陷段 | 「……上传时 **android 覆盖 windows**（v0.1.1/v0.1.2 均如此）」 | 保留「同名互相覆盖」的确定部分，**删除未经确认的方向断言**，改为「覆盖方向在 2026-09-13 复核中未能确认」，并给出两个实际资产（196 B / 88 B）与需取回原文才能定论 | GitHub API 资产 size/created_at/uploader；本记录 §2 |
| G-13 | `docs/RELEASE-STATUS.md` 证据目录 | 「`artifacts/plugin-acceptance-2026-09-06/`（summary.md + rerun-1a/ + rerun-1b/）」 | 补全为 `summary.md + channel-a/ + channel-b/ + windows/ + rerun-1a/ + rerun-1b/` | `Get-ChildItem artifacts/plugin-acceptance-2026-09-06 -Directory` → 实际存在 channel-a、channel-b、windows、rerun-1a、rerun-1b |

---

## 9. 改动文件清单

| 文件 | 是否入库 | 变更要点 |
| --- | --- | --- |
| `docs/RELEASE-STATUS.md` | 是（已跟踪，改动可见于 `git diff`） | 更新日期；门禁表第 1/4/5/12/14 行；整体状态；停止位置（G-01+G-11）；校验和缺陷段（G-12 疑似）；v0.1.2 节门禁段；证据目录（G-11+G-13） |
| `README.md` | 是 | `:25`（G-04）、`:41`（G-03）、`:43`（G-02） |
| `docs/ARCHITECTURE.md` | 是 | `:3`、`:169`、`:174`、`:180-181`、`:187`、`:188`（G-05） |
| `docs/HANDOFF.md` | **否——被 `.gitignore:92` 忽略** | `:3`、`:26`（新增增补段）、`:30-31`、`:46`、`:79`、`:137`（G-05） |
| `docs/review-2026-09-13/fixes/07-goals-docs-fixes.md` | 否（同属未跟踪的 review 目录） | 本记录 |

校验：`git diff --stat` → `README.md | 7 +++----`（+3/−4）、`docs/ARCHITECTURE.md | 14 +++++++-------`（+7/−7）、`docs/RELEASE-STATUS.md | 32 ++++++++++++++++++++------------`（+20/−12）；HANDOFF 因被忽略不出现在 diff 中。四个文档行尾均为纯 LF（无混合行尾），本记录同样为 LF。

---

## 10. 需 Lead 决策项

1. **是否为 v0.1.2 之后版本补生成 `SHA256SUMS-windows.txt`**：本任务采用「改文档口径」方案。若 Lead 选择改 workflow（`.github/**` 不在我写作用域），需同时修改 Release 正文模板中的校验和说明与 Android 下载通配符（见第 2 项），否则每个版本仍要靠 README 兜底。
2. **`.github/workflows/release.yml` 生成的 Release 正文仍写 `*-arm64-*-release.apk`**（与 G-03 同一错串），且正文宣称的校验和文件名与 v0.1.2 实际资产不符。属 CI 文件，需 Lead 或后续任务处理。
3. **`docs/HANDOFF.md` 被 `.gitignore:92` 忽略**：本次对其回写只存在于本机工作区，不会进入任何 commit/远端。若希望交接文档入库，需要 Lead 决定（改 `.gitignore`、或另建入库版）。
4. **`.gitea/workflows/*` 三份工作流仍留在库中且仍被 Pester 合同测试引用**（G-06）：是否归档/删除、以及 Pester 断言是否改为「历史合同（已停用）」，涉及 CI 与测试改动，超出本任务写作用域。
5. **是否需要把「远端 CI 只构建、门禁只在本地」同步写进 `docs/release-closure-2026-09-06.md`**（该文件不在本任务写作用域；目前只在 RELEASE-STATUS 与 ARCHITECTURE 说明）。

## 11. 遗留与未覆盖

- **G-12 方向未定论**：需在可下载 Release 资产的环境取回 `SHA256SUMS.txt` 原文（196 B / 2 行可核对产物名）后才能给出结论；本次只把未验证的断言改为「未确认」。
- **APK release keystore 签名缺少可离线复现证据**（无 `apksigner verify`）：已在 `ARCHITECTURE.md` 技术债 3 与 `HANDOFF.md` 剩余缺口/下一优先级中标注为待补；本任务不新增签名验证步骤。
- **未改动的陈旧项（属其它任务范围）**：`ROADMAP.md` Phase 6 §11.3 指引与 Phase 7 无状态（G-10/G-14）、`DOCS-CODE-AUDIT.md` 命令分布、`ST-EVENTS-COVERAGE.md` 事件计数、`CLAUDE.md`/`DATA_MODEL.md`/`FRONTEND-COMPONENTS.md` 与修复后代码对齐 → 按 Lead 排期由 **task-16** 处理（本次未触碰）。
- **`ARCHITECTURE.md:175`** 仍写「Windows bundle、Android APK、签名、GUI 和真机证据必须在 `docs/RELEASE-CHECKLIST.md` 单独记录」——历史口径（当前权威入口是 `RELEASE-STATUS.md`），本次未改，留待 task-16 统一。
- **本轮只改文档，未运行任何门禁**：Markdown 无 lint 步骤；若 Lead 需要，可在门禁外增加一次文档链接/一致性检查（本次改动均为文本替换，未新增外链）。
- 提醒：代码修复完成后请派发 **task-16**（CLAUDE.md / DOCS-CODE-AUDIT / DATA_MODEL / FRONTEND-COMPONENTS 等与修复后代码对齐）。

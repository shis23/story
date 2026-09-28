# 全量修复后确定性门禁报告（GATE-REPORT）

- 日期：2026-09-13
- 执行人：**Lead（独占门禁）**——所有命令由 Lead 亲自运行，日志落盘可复现
- 工作树状态：**未提交的工作树**（本轮不做 git commit；`docs/review-2026-09-13/**` 亦未入库）
- 改动规模：`git status --porcelain` 197 行（已跟踪修改 177 / 未跟踪 10 / 删除 10）；`git diff --stat` = **187 files changed, +14438 / −2471**
- 日志目录：`artifacts/review-2026-09-13-round2/`

> 说明：本报告只记录**真实运行结果**。前端命令在受限沙箱下由成员侧运行会报 `spawn EPERM`（环境限制，非项目缺陷），因此**全部前端门禁由 Lead 在非受限上下文运行**，成员记录里凡标注"未经 npm/vitest/build 验证"的改动，均以本报告为最终验证结论。

## 1. 门禁总表（最终一次全量运行）

| # | 命令 | 退出码 | 关键计数 | 首轮基线 | 对比 |
|---|------|--------|----------|----------|------|
| 1 | `cargo fmt --all -- --check` | **0** | 无 diff | 0 | 持平 |
| 2 | `cargo clippy --workspace --all-targets -- -D warnings` | **0** | 0 error / 0 warning | 0 | 持平（过程中曾 101，见 §3） |
| 3 | `cargo test --workspace` | **0** | **99 套件 / 2165 passed / 0 failed / 33 ignored** | 1980 passed / 33 ignored | **+185 passed**，ignored 持平 |
| 4 | `cd frontend && npm test`（node --test） | **0** | **532 tests / 532 pass / 0 fail** | 504 passed | **+28 通过** |
| 5 | `cd frontend && npm run test:ui`（vitest） | **0** | **31 files / 151 tests / 151 passed** | 27 files / 119 tests | **+4 文件 / +32 用例** |
| 6 | `cd frontend && npm run build` | **0** | `✓ built in 3.78s` | 通过 | 持平 |
| 7 | `node scripts/architecture/backend-baseline.mjs` | **0** | 定义 175 / 注册 175 / 前端唯一 invoke 152 / 孤儿 23 | 门禁未成形（首轮为只读统计） | 升级为**门禁**（违规 exit 1） |
| 8 | `powershell -NoProfile -ExecutionPolicy Bypass -File scripts/tests/run-release-build-tests.ps1`（Pester） | **0** | **198 passed / 0 failed**（51 + 14 + 34 + 99 四个 suite） | 198 套 | 持平（期间曾红，见 §3） |

日志对应文件：`fmt.log`、`clippy.log`、`cargo-test.log`、`fe-node-test.log`、`fe-vitest.log`、`fe-build.log`、`backend-baseline.log`、`pester.log`。

## 2. 本轮门禁把"红"修成"绿"的全部过程（诚实记录）

第一次全量门禁**不是全绿**，以下是每一次红灯、根因与修复者：

| 门禁 | 首次结果 | 根因 | 处理 | 最终 |
|------|----------|------|------|------|
| `cargo fmt --check` | exit 1（workspace 多文件未格式化） | 多域并发编辑后未统一格式化 | Lead 执行 `cargo fmt`（`fmt-apply*.log`） | 0 |
| `cargo clippy -D warnings` | exit 101（**5 error + 3 warning**） | 见下 §3 | Lead 逐条修（1 处交域2 的自动修复残留） | 0 |
| `cargo test --workspace` | 0 | — | — | 0 |
| `npm test` | 0 | — | — | 0 |
| `npm run test:ui` | exit 1（**2 文件 / 6 用例失败**） | 本轮新增的插件宿主安全测试：① M-24 握手令牌时序（**真缺陷**）② M-07 反例标记选错 ③ 自导航旧令牌可重放（**真缺陷**）④ M-31a `<style>` 正文残留（**真缺陷**） | task-27 → task-29（域6）+ Lead 收口（§3.2） | 0（31/31） |
| `npm run build` | 0 | — | — | 0 |
| `backend-baseline.mjs` | 0 | — | — | 0 |
| Pester | 198/0 | — | — | 198/0 |

**最有价值的一环**：`npm run test:ui` 的红灯不是测试噪音，而是**本轮新写安全测试抓出了三个真实缺陷**（插件桥握手整体失效、自导航后令牌可重放、插件 CSS 正文注入宿主 DOM）。详见 §3.2 与域6 记录 §14/§15。

## 3. Lead 在门禁期直接修复的条目

> 归属口径：以下改动由 Lead 在收口期完成；成员记录中相关条目标注为"Lead 收口修复"。

### 3.1 Rust 侧（clippy/fmt 归零，5 error + 3 warning）

| 位置 | lint | 处理 |
|------|------|------|
| `crates/app-agent/src/runtime.rs:611-617` | `clippy::empty_line_after_doc_comments` / doc list | W-01 的 helper 被插在 `spawn_subagents` 的 doc 与函数之间，把文档注释和 `#[allow(too_many_arguments)]` 隔断（该 doc 一度错误地挂在 helper 上）。**修复：把 doc 块与 allow 移回 `spawn_subagents` 正上方**（`#[allow]` 附理由：11 参数是流水线装配面的自然投影） |
| `crates/app-agent/src/runtime.rs:652` | `too_many_arguments (11/7)` | 同上，`#[allow]` 归位 |
| `crates/infra-sqlite/src/exporter.rs:1479` | `empty_line_after_doc_comments` | 删除 doc 与 `#[allow]` 之间的空行 |
| `crates/infra-sqlite/src/readiness.rs:1242` | `redundant_redefinition` | 删除 `let backup_dir = backup_dir;` 自赋值残留 |
| `crates/infra-sqlite/src/lease.rs:55` | `dead_code`（field `file` never read） | **不是死代码**：`HeldLease.file` 是 RAII 字段（持 fd 到最后一个 holder 退出才关闭，见该结构体上方的关键不变式）。保留字段 + `#[allow(dead_code)]` + 注释说明；**未删除** |
| `crates/tauri-app/tests/sqlite_meta_campaign_binding.rs:16/18/21` | doc list overindented | ①②③④ 续行缩进改为 2 空格 |

### 3.2 前端侧（`frontend/src/components/PluginHost.vue`，3 处真实缺陷）

| 缺陷 | 证据 | 修复 |
|------|------|------|
| **M-24 握手令牌时序 ⇒ 插件桥整体失效** | `iframeDoc`（computed）嵌令牌，`watch(iframeDoc)` 回调**先用已求值的 doc（旧/空令牌）注册**、之后才换令牌；`handshakeDocumentSequence` 又使 computed 变脏 ⇒ 自激重注册循环，注册文档永远落后一轮 ⇒ 桥脚本发 T_{n-1}、宿主期望 T_n ⇒ 握手永不成功 | 域6 按 Lead 诊断重构：抽出 `composeShellDoc(plugin, token)`；watch 源改为与令牌无关的 `handshakeSourceKey`；**先生成令牌 → 再组装 → 再注册**（每次源变化恰好注册一次） |
| **自导航后旧令牌可重放** | `onIframeLoad` 只重置 `handshakeValid`，令牌不变 ⇒ 重放旧令牌仍通过 `isPluginBridgeHandshakeValid` | Lead 加 `expectedFrameLoad`：宿主自身注册引发的那次 load 消费标记；**任何额外 load ⇒ 轮换令牌**（不能无条件轮换，初次加载也走 load） |
| **M-31a `<style>` 正文残留** | `FORBID_TAGS:['style']` 走 DOMPurify `KEEP_CONTENT` 语义：元素删掉、**CSS 正文留成裸文本**（`FORBID_CONTENTS:['style']` 在 3.4.12+happy-dom 实测无效），被 `v-html` 插进宿主 DOM 显示 | Lead 新增 `stripStyleElementsFromSlotHtml()`：sanitize **之前**用 `/<style\b[^>]*>[\s\S]*?<\/style\s*>/gi` 整块删除；无 `<style` 时逐字节不变；`FORBID_TAGS`/`FORBID_CONTENTS` 保留作纵深防御 |

**测试环境事实（务必记录，避免后人误判）**：
- vitest 的 happy-dom 下 **DOMPurify 3.4.12 的白名单解析与真浏览器不一致**（R4 复检更正本文早期"白名单为空"的过宽表述）：实测 `sanitize('<p>hi</p>') === 'hi'`、`sanitize('<span>hi</span>') === 'hi'`、`sanitize('<button …>ok</button>') === 'ok'`，但 `sanitize('<div><p>hi</p></div>') === '<p>hi</p>'`、`sanitize('<button>ok</button><style>x{}</style>') === 'ok<style>x{}</style>'`，且 `isSupported === true`。结论不变：**happy-dom 下不能依赖"元素/属性会被 DOM 保留"的断言**，必须打在纯函数上或在真 Chromium 里验证。M-31a 用例的控制组已改为断言 `stripStyleElementsFromSlotHtml` 的**精确输出**（环境无关，且非恒真）。域6 记录 §15.3 的措辞已按本条收窄。
- `N-R4-02`：`stripStyleElementsFromSlotHtml` 对**未闭合** `<style>`（含 HTML 中等价于开标签的 `<style/>`）原先不覆盖 ⇒ 真浏览器里 CSS 正文仍可能以裸文本落入宿主 DOM。**Lead 已补强**（从其位置截断）并补 3 条断言；本文档记录的是补强后的代码状态。

**Pester 门禁的宿主敏感性（N-R4-05，来自 R4）**：本报告的 198/0 是在 **Windows PowerShell 5.1（`powershell`）+ Pester 3.4.0** 下取得（日志头部 `Using Pester 3.4.0`）。R4 用 `pwsh`(PS7) 复跑时 `ReleaseBuild.Tests.ps1` 出现 43 passed/8 failed，8 条全为 `Should Throw` 的语义假红；裸 `powershell Invoke-Pester` 则会解析到 Pester 5.7.1 并因沙箱拒绝注册表驱动器而不可用。**复现基线的唯一可靠方式**：用 `powershell -NoProfile -ExecutionPolicy Bypass -File scripts/tests/run-release-build-tests.ps1`，不要用 `pwsh`。

**文档计数更正（N-R6-01，来自 R6）**：本文档早期引用的"27 个 docs 文件未入库"是**口径错误**（那是顶层 `git status --ignored` 条目数）。实测：`docs/` 下共 **199 个文件、其中 140 个被 `.gitignore` 忽略**。CLAUDE.md 的必读顺序已按 A 组（入库）/B 组（本机）分组缓解影响；`.gitignore` 本轮**未改动**（发布策略是用户决策）。

## 4. 门禁覆盖的"未验证项"（不得当成已验证）

1. **`.github/workflows/release.yml` 的 CI 改动（T-10：契约测试步驟、Windows 校验和文件、APK 通配修正）没有真实运行验证**——本地无法触发 GitHub Actions。静态结构由 Pester（99 项）覆盖；"能在 CI 上跑通"仍是**未验证**。
2. **release.yml 的 Actions 未被真实触发**；`SHA256SUMS-windows.txt` 等产物生成未经端到端验证。
3. **真 Chromium 行为**：本报告的前端结论基于 vitest + happy-dom；凡依赖真实浏览器 DOM/CSP/iframe 语义的条目（插件宿主、Card Shell、CSP 白名单）在 happy-dom 下只能验证到"逻辑层"，**真实 WebView 行为未在本轮验证**。
4. **Windows runner / Android release 签名**：无运行环境，维持首轮"未验证"口径。
5. **`harness-real-llm` 真模型用例**：本轮未运行（需真实模型与凭据），沿用 33 ignored。
6. **未提交工作树**：以上所有门禁跑在**未 commit 的工作树**上；`docs/review-2026-09-13/**`、`artifacts/**` 未入库。

## 5. 与审查结论的衔接

- 首轮报告 194 条发现（P0=3 / P1=44 / P2=100 / P3=47）的逐条处置记录在各域 `fixes/*.md`（01–09）。
- 本轮门禁期间**新增**的真实缺陷（§3.2 三项）属于第二遍审查发现的 N 类新发现，将在 `FULL-REVIEW-REPORT` 第二版与 R 系列复检报告中列出（编号 N-05/N-06/N-07）。
- 门禁全绿 → 已解除 R1–R7 复检任务的阻塞：`task-18`/`task-19`/`task-20`/`task-21`/`task-22`/`task-23`/`task-26`。
- 复检阶段又开出一批**收口任务**（R8–R12），它们**不在本文档的门禁结论内**，但会改动被测对象：`task-31`（M-30 文档 + G-07 tag↔版本校验）、`task-32`（W-18/W-32）、`task-33`（W-31 前端档位）、`task-34`（**PostProcessSkipped 双发回归修复** + 域3 记录更正）、`task-35`（W-11 下游 fail-open 裁定 + N-R7-04）。但这些任务落地后**必须重跑本文档 §6 的全部命令**——最终一次全量门禁与计数以第二版总报告 `round2/FULL-REVIEW-REPORT-v2.md` 为准。

## 7. 本文档的修订记录（被复检指正后更正）

| 时间 | 更正项 | 依据 |
|------|--------|------|
| 复检后 | happy-dom/DOMPurify 表述由"默认白名单解析为空"收窄为"白名单解析与真浏览器不一致（附实测反例）" | `round2/R4-meta-docs-recheck.md` N-R4-01 |
| 复检后 | 补记 Pester 门禁的宿主敏感性（`powershell` 5.1 + Pester 3.4.0 才能复现 198/0；`pwsh` 会 8 条 `Should Throw` 假红） | N-R4-05 |
| 复检后 | "27 个 docs 未入库"更正为"`docs/` 199 文件 / 140 个被忽略" | `round2/R6-fix-completeness-audit.md` N-R6-01 |
| 复检后 | `stripStyleElementsFromSlotHtml` 增加"未闭合 `<style>` 截断"并补 3 条断言 | N-R4-02 |

## 6. 复现命令（逐条可直接粘贴）

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --message-format short -- -D warnings
cargo test --workspace
Push-Location frontend
npm.cmd test
npm.cmd run test:ui
npm.cmd run build
Pop-Location
node scripts/architecture/backend-baseline.mjs
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/tests/run-release-build-tests.ps1
```

预期：全部 exit 0；计数与本报告 §1 表格一致（测试数只增不减视为正常）。

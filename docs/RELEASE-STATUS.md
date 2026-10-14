# 当前发布状态

更新日期：2026-09-06（Asia/Shanghai；本轮从 9 月 5 日持续到次日），2026-09-13 按文档复核结论回写停止位置、校验和资产与远端 CI 范围；2026-09-28 落库审查轮成果并关闭 P0-2（子帧 IPC，见下）；2026-10-13 回写 10 月增量与最新一次 11 步门禁计数（见「2026-10-13 增量」）。版本：0.1.2。

本文是当前候选版本的公开验收入口。架构见 `ARCHITECTURE.md`，用户操作和备份边界见 `USER-GUIDE.md`，历史验收明细保留在 `RELEASE-CHECKLIST.md`。内部交接、审查材料和原始截图可能仅在本机保留，不应作为公开文档的唯一依据。

## 完成范围

本次收尾以现有发布范围为准：Campaign 写作、三种主界面生成模式、采纳/放弃/重跑、已采纳历史保护、末尾分支、v3 故事快照、ST 导入导出、Meta/MVU、默认 SQLite、Windows 和 Android 主流程。

任意历史回滚、完整应用云同步、ST 全量长尾 API、JSON 生产写路径删除、记忆参数标定和 CoT 长程研究不属于本次既定产品范围。不得把这些未实现能力写为已完成。

## 当前门禁

| 门禁 | 当前状态 | 通过条件 |
| --- | --- | --- |
| 源码和文档一致性 | 已回写（2026-09-13 复核） | 模式、快照边界、11 步命令、前置依赖及真实限制已同步；2026-09-13 复核发现并回写 HEAD/校验和资产/远端 CI 范围等陈旧声明，逐条依据（原声明 → 改后 → 证据）见 [`review-2026-09-13/fixes/07-goals-docs-fixes.md`](review-2026-09-13/fixes/07-goals-docs-fixes.md) |
| 密钥扫描、fmt、严格 Clippy | 本轮通过；2026-10-13 复跑通过 | 完整门禁通过，包含未跟踪构建输入扫描 |
| Rust 工作区 | 本轮通过；2026-10-13 复跑通过 | 默认并发：2026-10-13 为 2260 通过、0 失败、33 忽略（2026-09-06 三轮为 1980/0/33）；未声称忽略用例或覆盖率通过 |
| 前端逻辑、组件、CSP、移动布局、应用壳、构建 | 本轮通过；2026-10-13 复跑通过 | 2026-10-13 为 535 + 161 + 9 + 28 + 2 = 735 项（2026-09-06 三轮均为 504 + 119 + 9 + 28 + 2 = 662 项，计数完全一致，第三轮运行于已提交的 HEAD 树）；生产构建通过 |
| 发布脚本和工作流合同 | 本轮通过；2026-10-13 复跑通过 | 2026-10-13 为五套 Pester 共 208 项（新增 `ReleaseBuild.TagVersion.Tests.ps1` 10 项；2026-09-06 为四套 198 项），无失败或跳过；11 步统一入口已通过。范围说明：其中「工作流合同」断言仍读取 `.gitea/workflows/*`（ci-gates/release-host-evidence/windows-gates 三份），而 Gitea Actions 已于 2026-09-06 停用——该组断言认证的是一套已停用的 CI 配置，不等于 GitHub Actions 侧存在门禁 |
| Windows 系统凭据库 | 本轮通过 | 使用一次性凭据实际写、读、删；不只是内存 mock |
| Android 凭据生命周期 | 本轮真机通过 | 新写入、脱敏 DTO、重启解密、底层删除、旧明文迁移和再次重启解密 |
| ST 世界书往返 | 本轮通过 | JSON/SQLite 集成回归及真机导出 PNG 后重新导入保留两条世界书 |
| SQLite 并发和跨平台迁移 | 本轮通过 | 并发首次打开、既有导入并发测试通过；精确 LF/CRLF 白名单且拒绝篡改；真机升级前后 20 张表一致 |
| Windows 原生写作流程 | 本轮通过（真实模型） | 4948da7 构建 13/13 项真实 Tauri IPC 检查；LLM 为真实 glm-5.3-flash（coding v4 端点，SSE 透传），33 次调用全 200（25 流式+8 非流式），tool_mode=native 一次通过；L1 磁盘 connections.json 仅 SecretRef 引用、无明文 key，杀进程重启可读（详见下节） |
| Windows/Android 调试候选 | 本轮产出并验证 | Windows 原生运行；Android APK 签名校验及保留数据覆盖安装通过，仍是调试签名 |
| 正式分发安装包 | 已闭合（v0.1.2） | tag v0.1.2 触发 GitHub Actions release.yml（run 34013739416），windows/android/release 三 job 全部 success；GitHub Release v0.1.2 于 2026-09-06T05:42:43Z 正式发布（非 draft）；EXE/MSI/APK 三产物均下载实测 SHA-256 一致（APK 为 release keystore 签名）；校验和同名覆盖缺陷已补传 `SHA256SUMS-android.txt` 且 workflow 已改按平台命名——v0.1.2 Release 实际资产为 `SHA256SUMS.txt`（196 B，workflow 上传）与 `SHA256SUMS-android.txt`（88 B，发布后手工补传），**没有** `SHA256SUMS-windows.txt`，平台命名自下一版本生效；Windows EXE 仍无代码签名（详见下节） |
| 当前候选完整真机写作、第三方插件 | 已闭合（本轮通过） | Windows 原生真实 glm-5.3-flash 写作（上列行）；Android 真机 9/9 环节：GUI 表单配置连接（测试连接 4s 连通）、SecretRef 落盘、续写 91s、采纳记账、对手戏 4 段编排 10.5 分钟、杀进程冷重启全持久，正文真实生成（Campaign 56→603 字），全程未用 IPC 建连接；第三方插件两条通道：1B 真实 TavernHelper 卡 5/5 远程脚本（含 MVU bundle）执行成功，1A manifest 插件 4 个真实缺陷修复并复验。边界与证据见下节。**边界限定（2026-09-13，task-31／M-30-③；2026-09-28 更新）**：「通道闭合」指已验证的挂载／执行／权限门控路径在验收样本上闭合，**不等于不存在可绕过路径**——子帧／权限边界的运行时验证 P0-2（`M-01`）已于 2026-09-28 关闭：运行时 PoC 实锤后落地 `SUBFRAME_IPC_GUARD`（子帧内冻结 `__TAURI_INTERNALS__` + 封堵两条 IPC 传输），同法复验全部通过（证据 `artifacts/p02-poc/`，报告 [`review-2026-09-13/round3-p0-2-closure-2026-09-28.md`](review-2026-09-13/round3-p0-2-closure-2026-09-28.md)）；本行结论不构成插件沙箱攻击面评估（另见本文件 `:60` 的免责声明）。原「已闭合」判定不改写 |
| 当前提交的远端 CI | 已闭合 | 最终提交链 ce6117d（含插件修复与版本 0.1.2）与 tag v0.1.2 已推送 Gitea+GitHub 双远端；GitHub Actions tag 构建全绿（run 34013739416，即最终发布提交的远端构建证据）；另 main@4948da7 的 workflow_dispatch 调试构建全绿（run 33980646148）。Gitea Actions 已按用户 2026-09-06 决定停用：act_runner 与正在执行的任务容器已停止、排队 run 34-36 不再执行、仓库 has_actions 已关闭（原因为 runner 性能不足与工作区缓存 non-fast-forward，即 run #33 全败根因）；远端构建自本轮起只在 GitHub 进行。范围说明：release.yml 的三个 job 以构建与产物上传为主（windows/android：`npm ci` + `npm run build` + 一步前端契约测试 `node --test tests/tauri-command-contract.test.mjs`（2026-09-13 新增；2026-10-12 workflow_dispatch run 38267792406 的 windows/android job 已实跑通过，尚未经 tag 发版运行）+ `cargo tauri build --ci`；release：汇总上传），不含 fmt/clippy/`cargo test`/Pester——11 步确定性门禁只在本地 `scripts/verify-release.ps1` 执行；Gitea Actions 停用标记见 `.gitea/DECOMMISSIONED.md`。2026-10-12 补充：CI 钉 `ubuntu-24.04` 并把 actions 升到 Node 24 主版本（`6f6eef9`）后，workflow_dispatch run 38271134820（该提交）与 38267792406（main@`d031381`）windows/android 双绿，release job 按设计跳过；release.yml 只构建 Tauri 应用，不构建、不分发 `dsh-bridge` |

整体状态：**真实模型写作（Windows 原生 + Android 真机）、第三方插件样本、正式分发安装包（v0.1.2 已发布并实测核验三产物哈希）与远端 CI（GitHub tag 构建全绿，Gitea Actions 已停用）均已闭合；三轮完整 11 步门禁全绿且计数完全一致（第三轮运行于已提交的 HEAD 树，见「停止位置」）**。Gate 6 维持“关闭非 PASS”；不宣称任何覆盖率指标。命令、证据、候选哈希及失败过程见 [`release-closure-2026-09-06.md`](release-closure-2026-09-06.md)。不得将此状态改写成“整个项目已正式发布”。

停止位置：本轮收尾完成，无未完结验收。仓库 HEAD 为 `ab894c6`（`fix(release): platform-suffixed checksum files; record v0.1.2 closure`，2026-09-06 14:18:08 +08），已包含 SHA256SUMS 平台命名修复（`.github/workflows/release.yml` 与 `README.md`）；2026-09-13 复核时 `git status --porcelain` 为空，HEAD 已推送到 Gitea 远端（`origin/main` = ab894c6）；指向 GitHub 的本地 remote-tracking 引用 `github/main` 停在 `3b114fe`（2026-09-02），故本文件不声称 ab894c6 已推送到 GitHub。**发布用的 tag `v0.1.2` 指向 `ce6117d`**（插件修复 + 版本 0.1.2），即平台命名校验和修复不在 v0.1.2 发布物内，自下一版本生效。依赖目录完整（`npm ci` 与前端生产构建通过）。

**2026-09-28 增量（审查轮落库 + P0-2 关闭）**：2026-09-13~14 审查轮（194 条发现、二轮复评 R1–R14）整体落库为 `a891be3`（240 文件，+31120/−2622，含全套审查报告），落库前 11 步门禁全量复跑通过（日志 `artifacts/gate-2026-09-28/gate-run.log`，`GATE_EXIT_CODE=0`），`a891be3` 已推 Gitea+GitHub 双远端（2026-09-28 验证）。同日：P0-2/M-01 子帧 IPC 越权经运行时 PoC 实锤后以 `SUBFRAME_IPC_GUARD` 关闭（报告 `review-2026-09-13/round3-p0-2-closure-2026-09-28.md`）；tauri 升 2.11.6（GHSA-w28w-mhc8-qvjv）；R12-R3（JSON 直写路径孪生门禁 warn）收口；第二批改动门禁全量复跑绿（`artifacts/gate-2026-09-28-final/gate-run.log`）后落库 `e815334`。CI：workflow_dispatch 实跑验证——发现并修复上游 `setup-android@v3`/runner 镜像劣化（`61e9663`/`cd849c2`），run 36436534945 windows+android 双绿（release job 在 dispatch 模式按设计跳过；tag↔版本校验步骤仅 tag 构建执行，静态覆盖为 Pester，运行时验证待下一次真实发版）。harness-real-llm：33 ignored 中 24 通过（19 真模型 + 5 非模型，真实 glm-5.3-flash/gemini-3.8-flash，沙箱凭据不触碰用户配置），4 个受外部模型供给限制（中继已不路由 deepseek 系 + CF ~100s 非流式 524）、4 个按设计无法会话内跑完（endurance/eval 日级长跑）、1 个外部工具缺省跳过（记录 `review-2026-09-13/real-llm-run-2026-09-28.md`）。housekeeping：根目录垃圾文件 `nul` 删除；4 个被取代的 worktree 分支清理（HistoryScreen 重设计留 `archive/historyscreen-redesign-2026-08` 本地 tag）。2026-09-30 补充：P0-2 守卫在 **release 构建**上同法复验通过（round3 报告 §6.1 边界就此消除，证据 `artifacts/p02-poc/poc-evidence-after-fix-RELEASE.json`）；此前一晚的 GitHub 推送中断系 Clash 选中节点出口劣化，已切换节点修复，双远端同步至 `c67fd4b`。遗留的用户侧事项不变：API key 轮换（Gate 8 延后项）、四模型对比计划（待定题目）、中继模型路由恢复。

**2026-10-13 增量（10 月主线 + DSH 插件版 PoC 合入）**：

- 2026-10-11 全量审计修复：删除零引用代码（`180a033`）；infra 修复流式重试、SSE 错误、ST 正则与存储回滚（`6af3313`）；记忆召回范围收紧、派生向量索引统一维护、卡片世界书可编辑、嵌入模型改为可选（含嵌入配置与补建界面）、消息归档随分支与 Bundle 迁移（`5cef741`）。Tauri 命令 175 → 178（新增 `clear_embed_config` / `get_memory_index_status` / `rebuild_memory_embeddings`，无命令删除）。
- 2026-10-12 宿主无关的应用服务抽成 `crates/app-service`（`cd02e73`、`8ddb7cf`）：该 crate 不依赖 tauri；tauri-app 只保留启动与 `#[tauri::command]` 外壳，命令签名不变。
- 2026-10-12 写作质量：子 Agent 提示词不再带未替换的 `{name}`，开场白与历史展开 `{{user}}` / `{{char}}`（`79326ea`）；正文继承开场白风格——卡状态栏标签不判格式泄漏，沿用开场白的叙述人称与破折号（`1728462`、`adb60e2`、`0130a07`）。
- 2026-10-12 记忆机制真实模型实测后的修复：远记忆召回候选 3 → 15，避免被近期摘要去重吃光（`3ee48b3`）；消息归档摘要首行先列具体事实（`4f16b6f`，旧归档不重写）；后处理新任务的 `triggers` 写成纯文本时不再丢任务（`447eded`）；玩家告知的知识不再记为"来源无法解析"（`a059bc6`，放行/拦截判定不变）；压缩阈值环境变量 `STORYFORGE_COMPRESS_A_THRESHOLD` / `STORYFORGE_COMPRESS_B_THRESHOLD` 只用于实测时提前触发，生产不设（`7b58f24`）。实测记录仅在本机，不作为发布验收。
- 测试组织：infra-sqlite 与 harness-real-llm 的集成测试各链接为一个测试程序（`5cb4da1`），减少链接开销。
- DSH 插件版 PoC 合入 main（`93f9f9a`）：`crates/dsh-bridge`（stdio JSON-RPC sidecar，经 app-service 走生产写作路径，模型调用反向交给 DSH）+ `integrations/dsh-storyforge/`（DSH 插件）。**属实验性集成，不在本文件「完成范围」内**：release.yml 不构建也不分发它；PoC 使用 JSON 存储、所有管线角色共用一个模型、sidecar 没有 WebView（回复里完整闭合的状态栏 HTML 块经 DOMPurify 净化后渲染在 DSH 每轮回复下方，Shadow DOM 隔离样式，切块前去掉完整的 `<script>` 元素；卡片界面即卡壳在 DSH 浏览器里的 `sandbox="allow-scripts"` iframe 中运行（不给 `allow-same-origin`，壳文档由插件服务端拼并登记成带 CSP 响应头的一次性路径，信任规则与桌面相同；开场界面即首页 / 自定义开局只在开局前挂在开场白下方，选开场经 sidecar `campaign.apply_opening`、切换世界书条目启用经 `world_info.set_enabled`，与桌面 `apply_campaign_opening` / `set_campaign_world_info_enabled` 同一个 app-service 入口；壳自存的界面状态经 sidecar `card_shell.variables_set` 写回存档，与桌面 `set_card_shell_variables` 同一个 app-service 入口）；右栏另有卡内状态面板，即 MVU 原生渲染：手动分析、应用变量定义、状态行、交互按钮，写变量走与 Tauri `set_*_variable` 同一个 app-service 入口），并依赖 DSH 0.2.0-rc.1 的预览期接口。确定性覆盖：`cargo test -p storyforge-dsh-bridge`（含拉起 sidecar 的 4 个端到端测试程序，计入下述 Rust 工作区计数）与 `node --test integrations/dsh-storyforge/test/*.test.mjs`（60 项，**不在 11 步门禁内**，2026-10-14 单独运行通过）。真实 DSH + 真实模型的 PoC 验收记录仅在本机，不作为发布验收。
- 2026-10-14 MVU 分析器修复（桌面与 DSH 共用 `storyforge-app-meta`）：`emit_mvu_translation` 工具 schema 的对象数组补上 `items`，解析端把写成 JSON 字符串的数组元素解码回对象。此前经 DSH 用 gemini 原生工具调用时，模型把每个元素编码成字符串，分析静默降级成只有变量字段（DSH 插件档 2 验收中发现）；解析失败的日志现在带上工具参数的解析错误与开头。
- 2026-10-14 卡壳内联文档信任配对修复（桌面前端 `trustedInlineShellDocStarts`，DSH 插件经打包共用）：此前按出现顺序配对时，状态栏 `.load` 胶水里的 `<body><script>$('body').load(…)</script></body>` 也被当成内联文档、占掉卡正则的一次命中，同一条消息里排在它后面的合法内联界面因此退到确认卡（只会少放行、不会多放行）。现在按 `segmentShellContent` 认领出的内联段配对；`frontend/tests/card-shell-display.test.mjs` 补了回归断言（DSH 插件档 3 验收中发现）。
- 门禁：合并后的 main（`93f9f9a`）于 2026-10-13 完整运行 11 步 `scripts/verify-release.ps1` 通过——secret 扫描、Pester 五套 208 项、fmt、严格 Clippy、Rust 2260 通过/0 失败/33 忽略、前端 535+161+9+28+2=735 项、生产构建；日志 `artifacts/gate-2026-10-13/gate-run.log`（含 `GATE_EXIT_CODE=0`）。其后只改文档的提交未重跑门禁。
- 2026-10-14 DSH 插件卡片渲染档 2（MVU 面板，`a26e881`）与档 3（卡片界面 iframe，`4626dc3`）合入 main：合入前在与合并结果相同的树上完整运行 11 步 `scripts/verify-release.ps1` 通过——secret 扫描、Pester 五套 208 项、fmt、严格 Clippy、Rust 2264 通过/0 失败/33 忽略、前端 535+161+9+28+2=735 项、生产构建；日志 `artifacts/gate-2026-10-14-t3/gate-run.log`（含 `GATE_EXIT_CODE=0`）。DSH 插件自己的 `node --test` 49 项不在门禁内，合入前单独运行通过。
- 2026-10-14 卡壳界面状态落盘回归修复（`60cd944`、`7e24b6c`，合并提交 `f30753f`）：T-06 的变量写入键名守卫拒绝所有 `__` 键，桌面 `CardShellHost` 经 `set_campaign_variable` 写选择器桶 `__storyforge_card_shell_variables` 因此一直失败，壳的界面状态只留在 iframe 内。守卫不放宽，新增专用命令 `set_card_shell_variables`（只写该键，校验 `<类型>:<身份>` 桶形状与 2 MiB 上限，走同一活动 Turn 屏障），Tauri 命令 178 → 179；`__storyforge*` 键不进提示词注入、模板上下文与后处理变量目录，后处理对它们的写入丢弃，变量页不展示。合并提交上完整运行 11 步 `scripts/verify-release.ps1` 通过——secret 扫描、Pester 五套 208 项、fmt、严格 Clippy、Rust 2272 通过/0 失败/33 忽略、前端 535+164+9+28+2=738 项、生产构建；日志 `artifacts/gate-2026-10-14-bucket/gate-run.log`（含 `GATE_EXIT_CODE=0`）。
- 2026-10-14 DSH 插件的卡壳界面状态写回存档（`40bdf4e`，合并提交 `b4f9727`）：sidecar 新增 `card_shell.variables_set`，调用上一条的 `set_card_shell_variables_for_backend`；插件按存档串行读改写、合并落盘，不等落盘就回复界面。合入前单独运行 `cargo clippy -p storyforge-dsh-bridge --all-targets -- -D warnings`、`cargo test -p storyforge-dsh-bridge`（含新增的「重启后仍在」端到端用例）与插件 `node --test` 53 项通过；Rust 改动只在 dsh-bridge，未重跑 11 步门禁；没有在真实 DSH 里用真卡复验。
- 2026-10-14 DSH 插件开场界面与世界书（分支 `feat/dsh-opening-shell`：`b766d87`、`9c483d4`、`7581874`）：可信的首页 / 自定义开局界面只在对话只有开场白时挂在开场白下方，写过一轮后关闭；选开场只认插件挂过的开场界面，经 sidecar `campaign.apply_opening` 走桌面 `apply_campaign_opening` 的 app-service 入口；界面经酒馆助手世界书接口读条目名与启用状态、按名切换启用（sidecar `world_info.list` / `world_info.set_enabled`，app-service 新增 `list_campaign_world_info_for_host` / `set_campaign_world_info_enabled_for_host`，Tauri 两个命令改为委托、签名不变）。自定义开局提交用的 `createChatMessages` / `/trigger` 两端都不支持。合入前在分支上完整运行 11 步 `scripts/verify-release.ps1` 通过——secret 扫描、Pester 五套 208 项、fmt、严格 Clippy、Rust 2274 通过/0 失败/33 忽略、前端 535+164+9+28+2=738 项、生产构建；日志 `artifacts/gate-2026-10-14-opening/gate-run.log`（含 `GATE_EXIT_CODE=0`）。插件 `node --test` 60 项单独运行通过；真实 DSH 复验见下一条。
- 2026-10-14 命定之诗真卡在隔离 DSH 实例里复验开场界面（gemini-3.8-flash-high）：首页 DLC 开关与核心选择在「下一步」/「开始旅程」时经 `updateWorldbookWith` 写回存档（核对存档世界书一致）；选《来自异世界的"勇者"》后开场白改写、尾部补出新开场正文并挂上状态栏界面，接着写一轮，执笔请求里是新开场（不含 `【首页】`）。复验发现两端共有的两个问题并修复：卡片界面是不透明源，读 `localStorage` 抛 `SecurityError`，自定义开局界面整块空白——兼容层 `createCardShellRuntimeCompatibilityScript` 改为在存储不可用时换成每份文档的内存 Storage；自定义开局在模块顶层读全局 `VueRouter`——桌面 `CardShellHost` 与 DSH 插件的预加载补上 Vue Router 4.5.1（桌面加载失败只告警）。修复后自定义开局界面完整渲染；它最后「踏上旅程」用的 `createChatMessages` + `/trigger` 两端仍不支持。前端 `npm test` 536、vitest 164、CSP Chromium 9、生产构建与插件 60 项通过。
- 未覆盖：main@`93f9f9a` 尚无 GitHub 远端构建；未发新版本，版本号仍为 0.1.2。Windows 原生真实模型写作、Android 真机、第三方插件通道等真机验收没有在 10 月改动后重跑，上表对应各行仍是 2026-09-06 的结论。

三轮完整 11 步门禁均通过且计数完全一致（secret 扫描、Pester 四套 198 项、fmt、严格 Clippy、Rust 1980 通过/0 失败/33 忽略、前端 504+119+9+28+2=662 项、生产构建全部通过）：

- 第一轮（插件修复 + 版本 0.1.2 最终树）2026-09-06 13:02，`scripts/verify-release.ps1` 退出码 0（日志含 `GATE_EXIT_CODE=0`），日志 `artifacts/release-gate-2026-09-06/gate-run.log`；
- 第二轮（SHA256SUMS workflow/README 修复后，14:07）日志 `artifacts/release-gate-2026-09-06-fixsums/gate-run.log`；
- 第三轮（提交后 HEAD 树，14:26）日志 `artifacts/release-gate-2026-09-06-final/gate-run.log`；后两轮日志以 `Release gate passed.` 结束，未写入退出码行。

历史 PASS 不代表后续新改动已验收。

真机另有一局由旧版 v2 Bundle 导入的历史数据缺少源角色卡。当前版本明确拒绝将其导出为完整 v3 快照；原数据库保留，不能凭空补造已丢失资料。此历史数据限制不因新 Bundle 往返测试通过而消失。

## 2026-09-06 真实模型与插件闭合

本轮把此前全部未闭合门禁闭合。按保守口径记录，失败与边界一并保留。

### 真实模型写作（Windows 原生 + Android 真机）

- Windows 原生候选（HEAD 4948da7 构建，EXE SHA-256 前缀 `3A2867F8…CB84`）：13/13 项真实 Tauri IPC 检查全部通过。LLM 为真实 glm-5.3-flash（open.bigmodel.cn coding v4 端点，经本机记录代理，SSE 透传）：33 次调用全 200（25 流式 + 8 非流式），流式首 token 中位 2207ms，单调用中位 70.9s，token 下界 144,854（其中 reasoning 85,916）。tool_mode=native 一次通过。L1 磁盘 `connections.json` 仅 `storyforge-secret:v1` 引用、无明文 key；杀进程重启后 `get_active_connection` 可读。
- Android 真机（Redmi 23117RK66C，Android 16，APK SHA-256 `27a1184d…4ede12` 与候选一致）：9/9 环节通过——GUI 表单配置连接（测试连接 4s「连通成功 glm-5.3-flash」）、SecretRef 落盘校验、续写生成 91s、采纳记账（3 项）、对手戏第二轮 10.5 分钟（4 段编排）+ 采纳（4 项）、杀进程冷重启后连接/正文/会话全持久。正文真实生成（Campaign 56→603 字）。全程未用 IPC 建连接。
- 口径说明：Android 验收所用 APK 为 `bb39b43` 代码（与 4948da7 同代码，仅差 docs）；本轮后续插件修复（7 文件）未含在该 APK 内。插件修复不触及写作/连接/存储路径，由确定性门禁覆盖；修复后的完整 11 步门禁重跑结果：**通过（2026-09-06 13:02，`scripts/verify-release.ps1` 退出码 0）：secret 扫描、Pester 四套 198 项、fmt、严格 Clippy、Rust 1980 通过/0 失败/33 忽略、前端 504+119+9+28+2=662 项、生产构建全部通过；日志在 `artifacts/release-gate-2026-09-06/gate-run.log`**。

### 第三方插件（两条通道）

- 通道 1B（卡内真实 TavernHelper 脚本）：真实卡「命定之诗与黄昏之歌 v4.1」GUI 导入（原生文件对话框 Win32 直驱），5/5 远程脚本（含 MagVarUpdate bundle.js via jsdelivr）全部执行成功，MVU 变量初始化并在真实 LLM 轮后更新，开场轮采纳。PASS。
- 通道 1A（manifest 插件）：首轮验收发现 4 个真实缺陷并全部修复+复验：
  1. `InstalledPluginDto` 缺 `entry_html`，iframe 永不启动——已修；GUI 安装的插件 iframe 真实启动。
  2. DOMPurify 默认剥 `<script>`，插件无载体——已修（`ADD_TAGS script`）；安全边界在 sandbox iframe + 权限门控桥，注释已论证。
  3. 审计脱敏非幂等导致 ring buffer fail-closed 在 1 条——已修；6/6 入库 integrity valid。
  4. 事件广播 postMessage 传 Vue reactive 对象抛 DataCloneError（一轮 1003 条）——已修（payload 统一 JSON 往返解克隆）；复验 events=20、0 DataCloneError（**2026-09-13 复核标注：该复验出自 `artifacts/plugin-acceptance-2026-09-06/rerun-1b/`**——`summary.md`、`r5-plugin-frame-final.json`、`r4b..r4f-*-state.json` 的 console 收集窗口）。
- 复验硬证据（**2026-09-13 复核标注证据来源**）：prompt hook 标记 `[plugin-ok]` 进入生成轮**全部 4 类上游请求**（主生成/质量修订/摘要/后处理）——证据 = `rerun-1a/`（`r3-proxy-analysis.json`、`r3-hook-evidence.txt`；该轮 iframe 计数为 hookCalls=6/changes=6）；`hookCalls=changes=15` 的证据 = `rerun-1b/`；审计导出 integrity valid = `rerun-1a/r4-audit-export.json`。修复共 7 个文件（Rust 2 + 前端 5），新增单测 5 个（前端 504/504 绿）。

### 边界（不得夸大）

- 本轮证明真实第三方样本经两条通道加载并工作（脚本执行/变量/prompt hook/审计/事件），不等于 ST 99 事件全集、完整 TavernHelper 语义、密码学审计或插件沙箱攻击面评估。
- 如实记录：裸 IPC `start_writing` 不经过前端 hook 接线属设计行为（fail-open）；采纳后的记账后处理调用无 prompt hook（路径差异，未深究）；小票组件不自动刷新为已知 UI 细节。
- Gate 6 维持“关闭非 PASS”不变；不宣称任何覆盖率指标；历史 PASS 不代表后续改动已验收。

### 远端 CI（4948da7 阶段记录；已在 v0.1.2 闭合，见下节）

- main（4948da7）已推送 Gitea + GitHub 双远端。
- GitHub Actions release.yml workflow_dispatch 调试构建于 4948da7 全绿：run 33980646148（windows job 27m29s、android job 20m；产物 windows-artifacts 20.8MB / android-artifacts 12.4MB；android job 证明 4 个 ANDROID_* 签名 Secret 有效）。URL：https://github.com/shis23/story/actions/runs/33980646148
- Gitea ci-gates run #33 同一提交全部 job 失败，根因是 runner 工作区缓存 "Unable to pull refs/heads/v4: non-fast-forward update"（jd-linux-1 runner 环境问题，非代码问题；GitHub 同提交全绿佐证）。按既定决策远端构建以 GitHub 为准；Gitea runner 缓存需服务器侧清理，本轮不处理。

### 正式分发安装包（4948da7 阶段计划，当时未完成；已于 v0.1.2 闭合，见下节）

- 依赖已恢复：残留 vite/esbuild 进程占用清理后，node_modules 重装、npm ci、前端 build 通过。
- 计划：版本 0.1.2 + tag v0.1.2 走 GitHub Actions release.yml，出 Windows msi+nsis、Android arm64 签名 APK 与 GitHub Release。此处只写计划与依据，构建结果未发生，不预写成功；实际结果发生后补记。

### 正式分发与远端 CI 闭合（v0.1.2）

- 正式发布：tag v0.1.2 触发 GitHub Actions release.yml（run 34013739416），windows/android/release 三个 job 全部 success；GitHub Release v0.1.2 于 2026-09-06T05:42:43Z 正式发布（非 draft）。URL：https://github.com/shis23/story/actions/runs/34013739416
- 资产（三个产物均已下载实测，SHA-256 一致）：
  - `StoryForge_0.1.2_x64-setup.exe`：8,893,240 B，`7759fece23a3717dbc53d13e0e8b0a3db74869ace9a1adace031cf4f84ca6585`
  - `StoryForge_0.1.2_x64_en-US.msi`：12,226,560 B，`df8bf8c2b6d9da6b66beef0bc8644158ae2ef390b53d8e1cf95e94a41fcb6079`
  - `app-arm64-release.apk`：24,944,036 B，`75bfc0c5942c94f5b97a96bca18b278fc9aee9c72cbcaa5a1c904ac61ed804a4`（release keystore 签名）
- 校验和已知缺陷与处置：workflow 中 windows/android 两个 job 的校验和文件同名 `SHA256SUMS.txt`，上传时发生同名资产冲突（v0.1.1/v0.1.2 均如此）。**2026-09-13 复核定案：留存的是 Windows 那一份，缺的是 Android 那份。**依据：v0.1.2 的 `SHA256SUMS.txt` 为 196 B，恰好等于两个 Windows 产物名按标准 `sha256sum` 两行 CRLF 的字节数（2 × (64 + 2 + 30 + 2) = 196），而 Android 只有 1 个 APK、单行只能是 88 B（正等于 05:58 手工补传的 `SHA256SUMS-android.txt` 大小）；`ab894c6` 的提交说明亦记载「v0.1.1/v0.1.2 both shipped only the Windows sums; v0.1.2 got SHA256SUMS-android.txt uploaded manually as remediation」。据此，补救动作（补传 Android 校验和）与观测一致。workflow 已改为按平台命名（`SHA256SUMS-windows.txt` / `SHA256SUMS-android.txt`），自 v0.1.2 之后版本生效。
- 签名边界：Windows EXE 仍无代码签名（SmartScreen 警告，README 已说明）；Android 为 release keystore 签名。
- 远端 CI：最终提交链 ce6117d（含插件修复与版本 0.1.2）已推送 Gitea+GitHub 双远端；tag v0.1.2 已推双远端并在 GitHub Actions 全绿（上述 run，即最终发布提交的远端构建证据）；另 main@4948da7 的 workflow_dispatch 调试构建全绿（run 33980646148）。Gitea Actions 已按用户 2026-09-06 决定停用：jd 主机 act_runner 与正在执行的任务容器已停止、排队 run 34-36 不再执行、仓库 has_actions 已关闭；原因是该 runner 性能不足且存在工作区缓存 non-fast-forward 问题（run #33 全败的根因）。远端构建自本轮起只在 GitHub 进行。
- 三轮完整 11 步门禁计数完全一致：Rust 1980 通过/0 失败/33 忽略、前端 504+119+9+28+2=662、Pester 198、secret 扫描/fmt/严格 Clippy/生产构建通过。第一轮（插件修复 + 版本 0.1.2 最终树，13:02）日志 `artifacts/release-gate-2026-09-06/gate-run.log`，含 `GATE_EXIT_CODE=0`；第二轮（SHA256SUMS workflow/README 修复后，14:07）日志 `artifacts/release-gate-2026-09-06-fixsums/gate-run.log`；第三轮（提交后 HEAD 树，14:26）日志 `artifacts/release-gate-2026-09-06-final/gate-run.log`——后两轮以 `Release gate passed.` 结束，未写入退出码行。

### 证据目录

- `artifacts/realmodel-2026-09-06/windows/`（gitignored）
- `artifacts/realmodel-2026-09-06/android/`
- `artifacts/plugin-acceptance-2026-09-06/`（summary.md + `channel-a/` + `channel-b/` + `windows/` + `rerun-1a/` + `rerun-1b/`）
- `artifacts/release-gate-2026-09-06/gate-run.log`、`artifacts/release-gate-2026-09-06-fixsums/gate-run.log` 与 `artifacts/release-gate-2026-09-06-final/gate-run.log`（三轮完整 11 步门禁日志；第三轮运行于已提交的 HEAD 树）

## 已有证据

- 2026-09-06 真实模型/插件/正式分发 v0.1.2/远端 CI 闭合记录：见上节「2026-09-06 真实模型与插件闭合」；明细另见 [`release-closure-2026-09-06.md`](release-closure-2026-09-06.md) 追加章节。

- 2026-09-05 桌面架构修复记录：删除未采纳稿件、已采纳前缀保护、末尾分支、v3 快照、模式无关 Editor 修订和隔离 Card Studio Store 已实现。历史记录为 Rust 1974 通过、33 忽略及 13 项真实 Tauri IPC 检查；模型为本机协议夹具。
- `android-topbar-acceptance-2026-09-05.md`：真机安全区、顶栏、键盘、面板切换及无清数据覆盖安装。它不是凭据或真实模型写作验收。
- `frontend-polish-2026-09-05.md`：响应式布局、动效、侧栏折叠和 Windows 原生检查。
- `theme-palettes-2026-09-05.md`：四套配色及浅/深模式；历史记录 655 项前端自动化通过。
- Gate 6：Canary3/Coverage12/TextFallback3/Stability30 的历史真实模型证据已封存；Full100 未完成，2026-08-31 已决议关闭但不是 PASS，不再作为本次前置续跑。

## 数据与权限

验收使用隔离数据和合成卡，不清空用户数据库，不绕过迁移校验，不用明文凭据替代系统安全存储。新发布证书、付费模型的大规模消耗、对外推送和发布需要相应授权及可用条件。

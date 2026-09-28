# StoryForge 全量审查报告（第二遍 · v2）

- 日期：2026-09-13
- 基线：`HEAD ab894c6` + 本轮**未提交**修复工作树（`git status --porcelain` 197 行；`git diff --stat` = 187 files changed, +14438/−2471）
- 第一遍：`FULL-REVIEW-REPORT.md`（194 条：**P0 3 / P1 44 / P2 100 / P3 47**）
- 第二遍分域报告：`round2/R1..R7-*.md`（7 份，全部由**未参与第一遍同域修复**的成员独立完成——R1↔域3、R2↔域1/2、R3↔域4/5、R4↔域6/7、R5 对抗性、R6 对账、R7 接缝）
- 修复记录：`fixes/01..09-*.md` + `fixes/GATE-REPORT.md`
- 门禁日志：`artifacts/review-2026-09-13-round2/`

---

## 1 一句话结论

**第一遍的 194 条已全部处置（R6 审计：已修复 159 / 降级 20 / 非问题 1 / 暂缓 10 / 无记录 4，谎报 0），4 条"无记录"已逐条收口；但第二遍审查证明"修完"不等于"修对"**：R5 的对抗性复验显示三条 P0 中 **2 条已关闭、1 条（P0-2 子帧 IPC）仍开放**；R1/R2/R7 分别在存储守卫、写作流水线、跨域接缝上抓出**本轮修复自身引入的回归与过度外扩**（P1：S-01 守卫拒掉合法老数据、取消时 `PostProcessSkipped` 双发），**全部已修复并配上失败可控的回归测试**（R9–R14）。

**门禁全绿 ≠ 审查通过**——这是第二遍最重要的结论：本轮被新测试抓出的 3 个真实前端缺陷、R1 的 P1 启动阻断、R2 的双发事件，**都通过了 clippy 与当时的全部既有测试**。

**收口后的最终门禁**（Lead 实跑，见 §2.2）：fmt 0 / clippy 0 error / `cargo test --workspace` **99 套件 2184 passed 0 failed 33 ignored** / `npm test` **533** / vitest **32 文件 157 用例** / `npm run build` ✓ / 基线门禁 exit 0 / Pester **208 passed 0 failed**。

---

## 2 确定性门禁（Lead 独占执行，全程真实运行）

| # | 命令 | 退出码 | 关键计数 | 第一遍基线 |
|---|------|--------|----------|-----------|
| 1 | `cargo fmt --all -- --check` | **0** | 无 diff | 0 |
| 2 | `cargo clippy --workspace --all-targets -- -D warnings` | **0** | 0 error / 0 warning（过程中曾 101：5 error + 3 warning） | 0 |
| 3 | `cargo test --workspace` | **0** | **99 套件 / 2165 passed / 0 failed / 33 ignored** | 1980 passed |
| 4 | `cd frontend && npm test` | **0** | **532 tests / 532 pass / 0 fail** | 504 |
| 5 | `cd frontend && npm run test:ui` | **0** | **31 files / 151 tests / 151 passed**（首跑 2 文件 6 用例红） | 27 文件 / 119 |
| 6 | `cd frontend && npm run build` | **0** | `✓ built in 3.78s` | 通过 |
| 7 | `node scripts/architecture/backend-baseline.mjs` | **0** | 定义 175 / 注册 175 / 前端唯一 invoke 152 / 孤儿 23 | 只读统计 |
| 8 | `scripts/tests/run-release-build-tests.ps1`（Pester） | **0** | **198 passed / 0 failed**（51+14+34+99） | 198 |

**复现口径（必须遵守）**：第 8 条用 **Windows PowerShell 5.1 + Pester 3.4.0** 才能复现 198/0；`pwsh`(PS7) 会出现 8 条 `Should Throw` 语义假红，裸 `powershell Invoke-Pester` 会解析到 Pester 5.7.1 并被沙箱拒绝（N-R4-05）。

详细过程、5 条 clippy 归零、以及门禁期修掉的 3 个真实前端缺陷见 `fixes/GATE-REPORT.md`。

### 2.1 收口任务之后的复跑（Lead 实跑）

收口任务（§6）会改动被测对象，因此**前端门禁已在前端写入冻结后复跑**（R10 的 `writing.js` 改动 + Lead 的 N-R4-02 补强之后）：

| 命令 | 退出码 | 计数（收口后） | 对比首轮 |
|------|--------|----------------|----------|
| `npm test` | **0** | **533 / 533 pass / 0 fail** | 532 → 533（R10 新增 1 例 node:test） |
| `npm run test:ui` | **0** | **32 files / 157 tests / 157 passed** | 31/151 → 32/157（R10 新增 6 例 vitest） |
| `npm run build` | **0** | `✓ built in 3.85s` | — |
| `run-release-build-tests.ps1`（Pester） | **0** | **208 passed / 0 failed**（51+14+34+99+10） | 198 → 208（G-07 新增第 5 套 10 例） |

**Rust 全量（`cargo fmt/clippy/test --workspace`）与 `backend-baseline.mjs` 在全部收口任务停止后由 Lead 重跑** —— 见下方 §2.2。

### 2.2 最终门禁复核（全部写入者停止后，Lead 最后一次实跑）

| # | 命令 | 退出码 | 最终计数 | 第一遍基线 |
|---|------|--------|----------|-----------|
| 1 | `cargo fmt --all -- --check` | **0** | 无 diff（首次复核曾 exit 1：R11 新增测试的注释对齐漂移 → Lead `cargo fmt` 收口） | 0 |
| 2 | `cargo clippy --workspace --all-targets -- -D warnings` | **0** | 0 error / 0 warning（首次复核 **exit 101：24 × `needless_borrow`**，全部在 R12 新增测试的 `&resolve/&group_member/&no_policy/&private/&open/&restricted` 调用点 → Lead 收口） | 0 |
| 3 | `cargo test --workspace` | **0** | **99 套件 / 2184 passed / 0 failed / 33 ignored** | 1980 passed |
| 4 | `cd frontend && npm test` | **0** | **533 / 533 pass / 0 fail** | 504 |
| 5 | `cd frontend && npm run test:ui` | **0** | **32 files / 157 tests / 157 passed** | 27 文件 / 119 |
| 6 | `cd frontend && npm run build` | **0** | `✓ built in 3.63s` | 通过 |
| 7 | `node scripts/architecture/backend-baseline.mjs` | **0** | 定义 175 / 注册 175 / 前端唯一 invoke 152 / 孤儿 23 | 只读统计 |
| 8 | `scripts/tests/run-release-build-tests.ps1`（Pester） | **0** | **208 passed / 0 failed**（51+14+34+99+**10**） | 198 |

日志：`artifacts/review-2026-09-13-round2/final-*.log`。**注意**：全部门禁跑在**未提交**工作树上；`docs/review-2026-09-13/**`、`artifacts/**` 未入库。

---

## 3 门禁期抓出的真实缺陷（第一遍没发现，第二轮新增）

这三个不是"风格问题"，是**功能性/安全性缺陷**，全部由本轮新写的安全测试在真实 vitest 下抓出：

| 编号 | 严重度 | 缺陷 | 影响 | 状态 |
|------|--------|------|------|------|
| N-05 | **P1** | `PluginHost.vue` M-24 握手令牌**在文档组装之后**才生成 ⇒ 注册给帧的文档永远带上一轮（首次为空）令牌，且 `handshakeDocumentSequence` 引发自激重注册循环 | 壳文档桥脚本发 T_{n-1}、宿主期望 T_n ⇒ **插件桥整体失效，所有插件消息被丢** | 已修复（域6 重构为"先生成令牌 → 再组装 → 再注册"）+ R4 用单进程 harness 独立实测"挂载恰好 1 次注册、令牌==被接受令牌" |
| N-06 | **P1** | 自导航后**旧令牌可重放**：`onIframeLoad` 只重置 `handshakeValid`、令牌不变 ⇒ 重放旧令牌仍通过校验 | 帧自我导航后仍可重新取得信任 | 已修复（Lead 收口：`expectedFrameLoad` 标记，额外 load 轮换令牌）+ R4 实测"自导航后重放被拒" |
| N-07 | **P2** | M-31a `<style>` **正文残留**：`FORBID_TAGS` 走 DOMPurify `KEEP_CONTENT`，元素删掉但 CSS 正文留成裸文本（`FORBID_CONTENTS:['style']` 实测无效） | 插件 CSS 文本以纯文本插入宿主 DOM（界面污染） | 已修复（Lead 收口：sanitize 前整块删除）+ N-R4-02 追加"未闭合 `<style>` 截断" |

**方法论价值**：这三个缺陷都**通过了 clippy、通过了既有全部测试**。它们是被"为已修问题补写可失败测试"这个动作抓出来的——这也是第二遍审查值得做的核心理由。

---

## 4 三条 P0 的最终状态（R5 对抗性独立复验）

R5 **不采信任何一方表述**：自建 rustc 探针、自建 legacy 夹具 + 真 `run_cutover` + 自己查磁盘、逐环节读依赖源码。

| P0 | 原始问题 | 最终状态 | 证据（R5 自测，非转述） |
|----|----------|----------|--------------------------|
| P0-1 | 卡壳 ES module 扫描在 CJK 邻接处 panic | **已关闭** | 新旧函数**同一探针、同一语料**：**旧版 12/20 panic，新版 0/20 panic**；增量证据：旧版还会**丢数据**（panic 前丢掉后文合法 URL）；仓库回归测试 4 条输入全部纳入语料并通过 |
| P0-3 | 不完整 legacy 目录 ⇒ 孤儿行静默丢弃 + 权威 marker 固化 | **已关闭（原始路径）**（残留 1 条变体 R5-02） | 自建 6 种布局 + 真 cutover：`cards.json` 缺失 ⇒ readiness 与 cutover **双 Err**、**无 DB 文件、无 marker、`inspect_marker = Absent`**；控制组（完整/空/无交叉引用）照常放行 ⇒ 守卫没误伤 |
| P0-2 | Windows 子帧可调用任意命令 | **仍开放（仅"收敛"）** | 源码级机制链：wry 在 Windows **无视 `for_main_frame_only`**（`wry-0.55.1/src/lib.rs:990`）→ 子帧拿到 IPC 引导脚本；wry 用**发信帧自身 URL** 作请求 URL（`webview2/mod.rs:896-910`）；壳源是**已注册自定义协议** ⇒ `is_local=true`；仓库无 `permissions/`、`acl-manifests.json` 无 `__app__` ⇒ `has_app_acl_manifest=false` ⇒ `webview/mod.rs:1823` ACL 分支**整段跳过**。域6 的守卫测试是"**通过即告警**"，不是阻断 |

**P0-2 的最小运行时 PoC 步骤**见 `round2/R5-p0-adversarial-reverify.md §3.3`（含"先证帧 URL 是 local"与负对照，以及若看到 `about:srcdoc` 就如实记"本机未复现"的分支）。
**诚实边界（R5 自述）**：P0-2 **没有**做机器级运行时 PoC（会话无法起 GUI），`args.Source()` 在子帧下的实际字符串属**高置信推断**；P0-1 只证明"panic 会发生 + tauri src 无 `catch_unwind` ⇒ 命令必然失败"，**是否终止进程未验**；P0-3 用的是 crate 公开 API + 自建夹具，未做端到端 startup 触发。

---

## 5 第一遍 194 条的处置对账（R6 独立审计）

R6 从 7 份分域报告机械提取 195 个条目标题，扣掉 `G-17`（判为非缺陷）后与第一遍 §2.3 的 194 **完全对齐**，再逐条把状态追到所属域记录，**并对"移交"项追到接收方确认是否真的落地**。

**五计数：已修复 159 / 降级 20 / 非问题 1 / 暂缓 10 / 无记录 4**
**谎报数 = 0**：抽查 20 条（P0/P1 占 12 条 = 60%）全部回代码/测试验真（含 `is_char_boundary`、`readiness.rs:194 ImportSourceIncomplete`、`contains("让我来")==0`、`Url::parse`、`RESERVED_VARIABLE_NAMESPACE`、`tool_center.rs` 已删除等）；`M-01`（P0 主体未修）与 `T-10`（CI 未运行时验证）**主动标注未完成**，属诚实降级。

**"无记录"4 条（本轮最重要的审计产出）及收口**：

| ID | 现象 | 收口任务 |
|----|------|----------|
| W-18 | big_scene 自动路由生产不可达；"移交域5+域2"后接收方零动作 | task-32（R9） |
| W-31 | `stores/writing.js` 的 `validGenerationModes` 仍含 `big_scene`（UI 只渲染 3 档） | task-33（R10） |
| M-30 | 文档漂移（`card_shell_clear_cache` 已接线、bundle `format_version`、RELEASE-STATUS 边界口径） | task-31（R8） |
| G-07 | `release.yml` 无 tag↔版本校验 | task-31（R8） |

R2 另发现 `fixes/08-docs-sync-fixes.md:123` 存在**虚假归属**（声称域5 记录已在前端修好 W-18/W-31，实际零命中），已并入 task-34 更正。

---

## 6 第二遍新发现与收口（R1–R7 → R8–R14）

### 6.1 本轮修复引入的回归与过度外扩（第二遍最有价值的产出）——**全部已收口，含失败可控测试**

| 编号 | 级别 | 问题 | 证据 | 收口 |
|------|------|------|------|------|
| **N-R1-01** | **P1** | S-01 的 Rule B **过度外扩**：把"所有 conversations"当 campaign 依赖 ⇒ 合法 legacy 布局（`cards.json` + `campaign_id:null` 的 conversations + 无 `campaigns.json`）被判 `ImportSourceIncomplete`，而启动默认走 SQLite ⇒ **非 Campaign 老用户升级后启动即失败** | R1 探针独立复现（报告附录 A）；`readiness.rs:199-227`、`storage_backend.rs:1911-1944` | task-36（R13）：Rule B 只统计 `campaign_id.is_some()`；补第 ⑤ 组布局"失败可控"测试；保留真缺失仍拒的正例 |
| **N-R2-01 类** | **P1** | 取消时 **`PostProcessSkipped` 发两次**（`app-pipeline/src/lib.rs:1628-1632` + `tauri-app/src/runtime_support.rs:51-57`），违反"**恰好一个**"契约（W-28 引入） | R2 逐条回源复读 | task-34（R11）：定单一权威 + 消除重复 + "恰好 1 个"回归测试 |
| R5-01 | P2 | 质量门禁残留：**引号内**的「我将为你」「我来为你」仍无条件 Error 且 `blocks_accept=true`（W-04 主诉已关闭） | R5 直调 `run_quality_gate` 探针 | 并入 task-34 |
| R5-02 / N-R1-02 | P2 | `cards.json` **存在但为 `[]`** + campaigns 全悬空 ⇒ **仍发布权威 marker + completed**（留悬空会话）；审计链路存在（warn + `record_backend_incident`），不是静默，但"不会再固化数据"只对"缺失"成立、对"空文件"不成立 | R5 6 种布局实测 + R1 独立判断（两人独立命中） | task-36 |
| N-R1-03 | P2 | S-06 承接未落地：`lib.rs:525` `copy_dir_recursive` 仍 `let _ =`（嵌套集合拷贝失败静默），S-01 守卫拦不住 | R1 回源码 | task-37（R14） |
| N-R7-01 | P3 | **W-01 归一在两侧不一致**：domain `campaign_runtime.rs:108` 用 Unicode `to_lowercase()`（且 name 不 trim）vs app-agent `runtime.rs:608` `trim()+eq_ignore_ascii_case()`；`"Ähre"/"ähre"`、`" Alice "/"Alice"` 判等相反，而 `runtime.rs:604` 注释自称"同一套语义" | R7 反证 | 并入 task-34（含跨 crate 一致性测试） |
| N-R1-04 | P3 | 守卫用 `Path::exists()`，权限错误被误报为"源不完整" | R1 | task-36 |
| N-R3-01 | P3 | `evaluateGates` 未纳入 `staleRetainedDeclarations` ⇒ 单跑脚本会 exit 0，只有契约测试拦得住 | R3（含负控） | **Lead 已修**（gate 现在会红） |
| N-R4-02 | P3 | `stripStyleElementsFromSlotHtml` 不覆盖**未闭合** `<style>` ⇒ 真浏览器仍可能泄漏 CSS 文本 | R4 | **Lead 已修** + 3 条断言 |
| N-R3-02..06 / N-R4-01..05 / N-R6-02..08 / N-R7-02..05 | P3 | 记录行号漂移、`DOCS-CODE-AUDIT.md:14` 的 169/172 语义写反、`03` 报告计数三方不一致、README/RELEASE-STATUS 仍是上轮门禁数字、Pester 宿主敏感性、happy-dom 白名单表述过宽 等 | 各 R 报告 | 见 §7 修正与本报告 §8 |

### 6.1b R6「无记录」4 条的收口结果

| ID | 收口结论 | 证据 |
|----|----------|------|
| **M-30**（P3 文档漂移，3 子项） | **已修复**（① 标注"已修复(task-16)"，代码复核确认 `InspectorDrawer.vue:18/:45` 已接线；② bundle `format_version` 经核对**代码自洽无缺陷**——JSON 导出 v2、SQLite 导出挂 `runtime` 后写 v3（`storage_backend.rs:1554/:1562` + 立即 `validate_runtime`）、导入接受 1..=3 且 v3 必须有 `runtime`（`bundle_runtime.rs:20`），已按事实回写 `ROADMAP.md:123` + `PLAN-ST-IMPORT-EXPORT.md:62/63/111`；③ `RELEASE-STATUS.md:29` **追加**边界限定，原声明未删） | `round2/R8-unrecorded-closure.md` |
| **G-07**（release.yml 缺 tag↔版本校验） | **已修复**：新增 `scripts/release-build/verify-tag-version.ps1`（tag 去 `v` ↔ `tauri.conf.json` + `Cargo.toml [workspace.package]` 双源比对；漂移/非法 tag/缺文件 fail-closed；PS5.1/7 兼容）+ `scripts/tests/ReleaseBuild.TagVersion.Tests.ps1`（**10 用例**，含匹配/不匹配/预发布/无 `v` 前缀四类），已接入 `run-release-build-tests.ps1:59`；`release.yml:80-86` 在 Build 之前加一步并按 tag 门控（`workflow_dispatch` 的 ref_name 是分支名，无门控会误失败）；**99 条静态契约加步后全绿、未改任何契约测试** | 同上；**CI 侧未运行时验证** |
| **W-31**（前端 `validGenerationModes` 含 `big_scene`） | **已修复**（方案 A）：`stores/writing.js` 的校验集**从档位目录派生**，`big_scene` 不再是合法档位 ⇒ 旧 `localStorage` 值回退 `continuation`、`setGenerationMode('big_scene')` 空操作且不写盘、**读取阶段不重写/不删除**用户偏好 blob；后端 `BigScene` 兼容面与 `rerollPolicy` 未动，非 Campaign 路径行为不变；顺带更正了 `useWritingScreenAdapter.test.mjs:70-71` 中**固化缺陷的旧断言** | `round2/R10-unrecorded-frontend.md`；变异测试证明失败可控（旧实现下 5/6 红） |
| **W-18**（big_scene 自动路由可达性） | **判定非问题（保留，不删）**：是**有书面契约的 API 级能力**——`WRITING-PIPELINE-V2-IMPLEMENTATION-2026-07-27.md:9/:66`「显式选择优先／**未传模式的 API 调用才进入自动路由**」；具体调用点 `commands/writing.rs:877`（`Option<GenerationMode>`）→ `:927-930` 路由与守卫；6 条测试名（含以 `generation_mode=None` 实跑该路径的 `start_writing_command_prompt_hook_messages_reach_mock_llm`）＋域层 11 条 ⇒ 删除前提"全仓无调用方"不成立 | `round2/R9-unrecorded-pipeline.md` |

### 6.1c 收口任务总表（R8–R14，全部完成）

| 报告 | 任务 | 处置 | 证据要点 |
|------|------|------|----------|
| **R8** | task-31 | M-30 已修复（文档 3 子项）；G-07 已修复 | 见 §6.1b；Pester **198 → 208**；99 条静态契约加步后全绿、未改契约测试；YAML pyyaml `Valid=True` |
| **R9** | task-32 | W-18 判定非问题（保留）；W-32 补测试 | W-18：书面契约 + 调用点 + 6 测试名（见上）。W-32：现状是 app-agent 唯一实现 + `#[inline]` 委托薄壳 ⇒ 新增"委托逐字节等价 + golden 字节锁（分区标题/空行/`keys.join(", ")`）+ 空包边界"测试；**失败可控实测**（改掉 app-agent 文案即红） |
| **R10** | task-33 | W-31 已修复（方案 A） | 见 §6.1b；顺带更正 `useWritingScreenAdapter.test.mjs:70-71` **固化缺陷的旧断言**；变异测试 5/6 红 |
| **R11** | task-34 | **双发回归已修复** + 4 项记录更正 + 门禁误杀收窄 + 归一统一 | 单一权威 = Tauri 持久化层（终态事件仲裁：非终态即时转发、终态截留兜底，`drop(proxy_tx)`→`forwarder.await` 确定性排空）；失败可控实测=关掉仲裁出现 **2 个终态**且 Tauri 的 Skipped 早于 Started。同类面一并收口：成功路径原 **2×Done**、持久化失败原 **Done→Failed**。W-24 条目错挂 → 更正**并删除死熔断机制**（零行为变更）；W-08/W-12/W-14/W-30 → 部分关闭（各含未关部分 file:line + 反证）；`recall.rs:134-135` 注释按 fail-closed 实现改写；R5-01 收窄为"非引号 + 写作任务线索"（同源 N-R2-14 一并收口，披露可回退）；**N-R7-01** 统一为 domain `normalize_instance_identity`（trim + Unicode 小写），两侧失败可控均实测（`Ähre/ähre`、`" Alice "/"Alice"`） |
| **R12** | task-35 | W-11 下游 fail-open **逐处裁定**（不是一律 fail-closed） | `false` = **放行**（谓词 `source_propagation_blocks`；调用者=整条预检 + 逐 target，宿主 `build_knowledge_mutations` 被 JSON 与 SQLite 两条批构建共用）。F1（广播无 `source_character_id`）**判定非问题**（`app-agent/src/prompts/postprocess.rs:34-38` 的广播形状本就不带它）；F2（源解析不出）**判定非问题**（上游明示"未命中不猜、留下游兜底"）；**F3（源无同文本条目）暂缓**——fail-closed 会把 `told_by_other` 的常见合法形态一起静默丢掉，正确修法 = 稳定标识（跨域）。**三处 fail-open 全部从静默升级为 warn 计数**（`PropagationGate{blocked, unresolved}`），blocked 语义逐分支不变；3 条回归锁（含"改 fail-closed 端到端广播会从 2 条变 0 条"） |
| **R13** | task-36 | N-R1-01（P1）**已修复** + N-R1-02 fail-closed + N-R1-04 已修复 | Rule B 收窄为"只统计 `campaign_id` 非空的 conversations"（新 helper `readiness.rs:160`）；新增第 ⑤ 组布局（`cards.json` + `campaign_id:null` 会话 + 无 `campaigns.json` ⇒ Ok/Completed 且会话真落库）**＋反向锁**（campaign 归属会话缺 `campaigns.json` 仍拒）；**失败可控实测**（反转判据 → 第 ⑤ 组必红）。N-R1-02 采纳 fail-closed：判据"cards 文件存在且 0 卡 + campaigns 非空"（证据：`delete_card` 级联时 cards.json 先写、campaigns.json 后写 ⇒ 正常完成不可能留下该形态）。N-R1-04：`PathPresence`（只认 `NotFound` 为缺失）+ 新错误 `ImportSourceUnreadable`，**8 个调用点**，含 `cutover`（stat 报错按"存在"处理 ⇒ 读不了绝不被判"全新用户"建空库） |
| **R14** | task-37 | S-06 承接**已修**（不是判非问题） | 旧实现：嵌套拷贝失败**连日志都没有** + 无条件打印"数据迁移完成" + 一次部分失败即**永久锁死**（跳过判据只看 3 个路径存在性）。新实现：`MigrationOutcome` 可判定返回值 + 逐条失败收集（相对路径）+ `error!` + 健康面 `legacy_dir_migration_incomplete` + `.migration_incomplete` 标记（拷贝前写、成功才撤 ⇒ 下次启动越过 latch 重试）；**命令签名零改动**。执行级证据：旧实现三轮启动后嵌套数据**最终落地=false**，新实现 `Incomplete+健康事件 → 重试 → Completed+撤销标记` **落地=true**；5 条失败注入测试（含"删掉标记后确实返回 `SkippedPopulated`"的反证） |

### 6.2 第二遍确认"修对了"的部分（正面证据）

- **R1**：域1 31 条 + 域2 25 条中 **确认关闭 53 / 部分关闭 1 / 未关闭 0 / 虚报 0**；D-01 独立复现（旧算术同型输入 panic=true，新 guard 边界正确）。
- **R2**：P1 六条（W-01..W-06）**全部真实落地且方向正确**；W-04 用两组输入验证（正常对白不报、真元描述仍 Error）。
- **R3**：T-01 **正向 + 3 组负控**（注入裸 `_invoke('r3_negative_control_missing')` ⇒ 门禁 exit1、契约测试 exit1；`--write-snapshot` 脏状态**拒绝写且快照 SHA256 不变**，干净状态写 175 条且与已入库 fixture 哈希相同）；20 个 wrapper + 9 个基元删除**零残留**；契约不变量 `152 ∪ 23 == 175` 成立；**N-R7-04（S-01 启动路径是否真被消费）判定"已接通"**并给出完整调用链。
- **R4**：M-24 三修复**闭环**（独立 harness 实测）；M-02 **不可绕过**（Rust 测试 + 自造 **37 组**走私/IP/大小写/端口用例 0 命中）；M-03/M-04/M-08 各目标测试 1/1；域7 `RETAINED_NO_FRONTEND_CALLER` 23 条经门禁 + 契约测试 11/11；密钥扫描收窄**未削弱**（仅停 1/9 条低置信度规则）。
- **R5**：T-02 **已关闭**（旧版 8 处 `let _ =`，其中 3 处在成功返回路径；新版 0 处真吞错；故障注入前提用独立探针证明）。
- **R7**：7 条跨域接缝 **6 条对上**（D-04 真实时钟、M-04 变量命名空间 Rust 侧真拒绝、M-08 权限 fail-closed、T-01 三集合关系、降级残留显式记录、M-24/M-31a 三侧一致）。

---

## 7 门禁期与本报告自身的更正（诚实性）

| 更正项 | 原表述 | 更正后 | 依据 |
|--------|--------|--------|------|
| happy-dom/DOMPurify | "默认白名单解析为空" | "白名单解析**与真浏览器不一致**（`<p>`/`<span>`/`<button>` 单独输入被去标签，但 `<div><p>` 组合保留 `<p>`）" ⇒ 结论不变：不能依赖元素保留行为做 DOM 断言 | N-R4-01 |
| docs 未入库规模 | "27 个 docs 文件未入库" | `docs/` 共 **199 个文件、其中 140 个被 `.gitignore` 忽略**（"27"是顶层 `--ignored` 条目数） | N-R6-01 |
| Pester 数字 | "198 全绿" | 在 **PowerShell 5.1 + Pester 3.4.0** 下为真；`pwsh`(PS7) 会 8 条假红 | N-R4-05 |
| M-01/P0-2 措辞 | 一度接近"已收敛=可接受" | **仍开放**，且在修好前，任何"插件/卡壳权限已闭合"的措辞不成立 | R5 §3 |
| S-06 定级 | P2（第一遍） | **上调 P1**：含"静默 + **永久**数据不可达"路径且有**假成功日志**（R14 §0/§1.2 三条行号依据），建议在最终报告按 P1 表述 | R14 §0；Lead 采纳 |
| R5 冻结报告的引用 | `R5-p0-adversarial-reverify.md:180` 等引用的测试名/行号在 R13 落地后已漂移（原 `..._is_skipped_with_audit` → `..._is_rejected`） | **保留 R5 原文不追改**（冻结件），勘误在此：以 R13 §0 与 `fixes/02-storage-fixes.md §13` 的现名/现行为准 | review-storage 回执 |
| 本报告首次门禁数字 | 曾以"2184/533/157/208"之外的旧值（2165/532/151/198）出现在早期草稿 | **最终以 §2.2 为准**（2184 / 533 / 157 / 208） | 末次实跑 |
| Lead 收口的两处门禁失败 | 首跑 `fmt --check`=1、`clippy`=101（24 error） | 均为**最后一批新增测试**的机械问题（注释对齐漂移 + `needless borrow`），Lead 修复后 fmt=0、clippy=0，并重跑 `cargo test --workspace`=0 | `artifacts/.../final-*.log` |

---

## 8 仍未闭合 / 明确暂缓（不做虚假结案）

### 8.1 唯一仍开放的 P0
- **P0-2 / M-01：Windows 子帧 IPC 越权** —— 见 §4。**已交付**：源码级机制链、最小运行时 PoC 步骤、4 条守卫测试（17/17 通过，"通过即告警"）、以及"无 app ACL 清单 ⇒ ACL 分支整段跳过"的确切位置（`webview/mod.rs:1823`）。
  **本轮不修的理由**：正确修法需要真实桌面会话做运行时验证（PoC 未执行），且当年裁决已明确"不能仅靠补 ACL 清单——那会在 `Origin::Local` 无法区分 subframe 的前提下把所有命令一起拒掉"。**下一步建议按 PoC 步骤实测后再定方案**。

### 8.2 明确暂缓（附理由，均已落进对应记录）
| 项 | 理由 | 建议 |
|----|------|------|
| N-03 应用内向量冻结解冻入口 | 事实核对后**不成立**：`storage_health_acknowledge`（`commands/diagnostics.rs:16`）→ `write_fence::unfreeze`，UI `StorageHealthGate.vue`（`AppV2.vue:801`）已在 | — |
| 20 条新孤儿后端命令 | 产品决策：**保留并声明**（登记进 `RETAINED_NO_FRONTEND_CALLER`，共 23 条） | 若要删除，按 `04-tauri-fixes.md §3` 的 8 步清单 |
| 前端 `InlineHtml` deferred/byte_len 取回通道 | 跨域脚手架，需产品确认是否真要暴露 | 立项再评估 |
| W-13 RoundSummary 受众隔离 / W-14 历史归档 `campaign_id` 回填 | Lead 裁定暂缓（产品决策） | 与 Campaign-first 路线一起排期 |
| `.gitignore` 未改动 | 发布策略属**用户决策**，本轮不擅自更改 | 建议把审查报告随提交入库（N-R6-01） |
| **W-18 的"UI 自动升档提示"** | 后端自动路由与成本确认是**保留的 API 能力**（有书面契约 + 6 条测试），真实 UI 恒传显式档位 ⇒ 用户看不到自动升档属**设计取舍** | 若要落地，需前端功能开发（区分"未选档"vs"选了 continuation" + 成本确认对话框 + 带建议档位重试）——**建议单独立项** |
| **R12-R3 孪生门禁观测** | `commands/writing.rs:1513-1565` 的 JSON 直写路径孪生门禁尚未加同样的 warn（约 10 行、零行为变化）。**本轮不做**：只为 legacy-only 路径加可观测性，代价是再开一轮改动 + 重跑全量门禁，收益低 | 授权后可作为独立小任务；默认路径（SQLite）已有 warn |
| **R12-R1 广播必填 `source_character_id`** | F1 残留面：无源广播无法做策略判定（当判为**非问题**，因文档形状本就不带该字段） | 若要彻底关闭，需域1/域3 协同改造广播协议 —— **建议单独立项** |
| **R12-R4 F3 miss 率** | "源无同文本条目"放行的真实发生率未知 | warn 上线后按 `reason` 统计再决定是否收紧（正确修法是稳定标识 `source_entry_id`，跨域） |
| **N-R1-02 的严格度** | R13 采纳 fail-closed（"cards.json 存在但 0 卡 + campaigns 非空 ⇒ 拒"）；Lead **采纳该裁定**：正常完成不可能产生该形态（写盘顺序 cards 先、campaigns 后），放行代价=整棵 campaign 树静默消失 | 若要改回"不阻断启动"，回滚点=`readiness.rs:271-279` |

### 8.3 未运行时验证（不得当成已验证）
1. `release.yml` 的 CI 改动（契约测试步骤、Windows 校验和、APK 通配、以及 task-31 新增的 tag↔版本校验）——本地无法触发 Actions；静态结构由 198 条 Pester 覆盖。
2. 真 Chromium/WebView 行为：插件宿主、Card Shell、CSP 白名单在 happy-dom 下只验证到逻辑层。
3. Windows runner / Android 签名：无环境。
4. `harness-real-llm` 真模型用例：未运行（需真实模型与凭据），沿用 33 ignored。
5. 门禁全部跑在**未 commit 的工作树**上；`docs/review-2026-09-13/**` 与 `artifacts/**` 未入库。

---

## 9 第二轮审查的方法学结论（可复用）

1. **"修复必须配一个可失败的测试"是本轮唯一真正有效的质量杠杆**：门禁期 3 个 P1/P2 真实缺陷、R1 的 P1 启动阻断、R2 的双发事件，全部由"为已修问题补写测试"或"独立复现"抓出，而 clippy/既有测试全绿。
2. **"移交"必须追到接收方**：194 条里有 4 条在"移交"后无人接（R6），另有 S-06 移交域4 后无动作（R1）。**"移交"不是处置**，只是待办的另一种说法。
3. **修复会引入新缺陷，且往往在与原缺陷相邻的边界上**：S-01 的 fail-closed 拒掉合法老数据、W-28 的取消映射引入双发事件、W-24 的条目答非所问——第二遍审查的价值主要在这里。
4. **对抗性复验要能证伪自己**：R5 的做法（新旧版本放同一探针、旧版必须 panic 才算复现、附带"前提验证"、明确写下没做的部分）应成为后续 P0 复核的标准。
5. **记录与实际不一致是系统性风险**：19 条 R 系列新发现里，记录/文档漂移类占多数（行号漂移、虚假归属、计数三方不一致、README 仍写上轮数字）。

---

## 10 交付物索引

| 类型 | 路径 |
|------|------|
| 第一遍总报告 | `docs/review-2026-09-13/FULL-REVIEW-REPORT.md` |
| 第一遍分域报告 | `docs/review-2026-09-13/01..07-*.md` |
| 修复记录（9 篇 + 门禁） | `docs/review-2026-09-13/fixes/01..09-*.md`、`fixes/GATE-REPORT.md` |
| **第二遍总报告（本文件）** | `docs/review-2026-09-13/round2/FULL-REVIEW-REPORT-v2.md` |
| 第二遍分域报告 | `docs/review-2026-09-13/round2/R1..R7-*.md` |
| 收口任务报告 | `docs/review-2026-09-13/round2/R8..R14-*.md`（R8 文档/CI、R9 流水线、R10 前端、R11 回归、R12 存储 fail-open、R13 readiness、R14 迁移吞错） |
| 门禁日志 | `artifacts/review-2026-09-13-round2/`（`final-fmt.log` / `final-clippy.log` / `final-cargo-test.log` / `final-fe-*.log` / `final-pester.log` / `final-baseline.log`） |

## 11 变更规模与提交状态

- `git status --porcelain`：197 行；`git diff --stat`：**187 files changed, +14438 / −2471**（首轮门禁时点）。收口任务（R8–R14）在此外追加：`crates/app-pipeline/src/{lib,quality_gate}.rs`、`crates/app-agent/src/runtime.rs`、`crates/domain/src/campaign_runtime.rs`、`crates/tauri-app/src/{runtime_support,production_postprocess,lib,lib_tests_startup}.rs`、`crates/infra-sqlite/src/{readiness,cutover,importer,error}.rs` + `tests/importer_diagnostics.rs`、`frontend/src/stores/writing.js` + 3 个前端测试文件、`scripts/release-build/verify-tag-version.ps1`、`scripts/tests/ReleaseBuild.TagVersion.Tests.ps1`、`.github/workflows/release.yml`、`docs/**`（含 `ROADMAP.md` / `PLAN-ST-IMPORT-EXPORT.md` / `RELEASE-STATUS.md`）。
- **本轮未做任何 commit**：全部门禁与全部结论都是针对**未提交工作树**得出的。`CLAUDE.md`（gitignored）已按 R11 事实再回写 W-28 段（终态事件单一权威 = Tauri 持久化层）。
- 后端命令总数仍 **175**（未删任何命令）；前端唯一 invoke **152**；孤儿命令 **23** 条全部登记为"声明的保留 API"（门禁 exit 0）。

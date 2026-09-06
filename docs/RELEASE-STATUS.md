# 当前发布状态

更新日期：2026-09-06（Asia/Shanghai；本轮从 9 月 5 日持续到次日）。版本：0.1.2。

本文是当前候选版本的公开验收入口。架构见 `ARCHITECTURE.md`，用户操作和备份边界见 `USER-GUIDE.md`，历史验收明细保留在 `RELEASE-CHECKLIST.md`。内部交接、审查材料和原始截图可能仅在本机保留，不应作为公开文档的唯一依据。

## 完成范围

本次收尾以现有发布范围为准：Campaign 写作、三种主界面生成模式、采纳/放弃/重跑、已采纳历史保护、末尾分支、v3 故事快照、ST 导入导出、Meta/MVU、默认 SQLite、Windows 和 Android 主流程。

任意历史回滚、完整应用云同步、ST 全量长尾 API、JSON 生产写路径删除、记忆参数标定和 CoT 长程研究不属于本次既定产品范围。不得把这些未实现能力写为已完成。

## 当前门禁

| 门禁 | 当前状态 | 通过条件 |
| --- | --- | --- |
| 源码和文档一致性 | 本轮更新 | 模式、快照边界、11 步命令、前置依赖及真实限制已同步 |
| 密钥扫描、fmt、严格 Clippy | 本轮通过 | 完整门禁通过，包含未跟踪构建输入扫描 |
| Rust 工作区 | 本轮通过 | 默认并发：1980 通过、0 失败、33 忽略；未声称忽略用例或覆盖率通过 |
| 前端逻辑、组件、CSP、移动布局、应用壳、构建 | 本轮通过 | 499 + 119 + 9 + 28 + 2 = 657 项；生产构建通过 |
| 发布脚本和工作流合同 | 本轮通过 | 四套 Pester 共 198 项，无失败或跳过；11 步统一入口已通过 |
| Windows 系统凭据库 | 本轮通过 | 使用一次性凭据实际写、读、删；不只是内存 mock |
| Android 凭据生命周期 | 本轮真机通过 | 新写入、脱敏 DTO、重启解密、底层删除、旧明文迁移和再次重启解密 |
| ST 世界书往返 | 本轮通过 | JSON/SQLite 集成回归及真机导出 PNG 后重新导入保留两条世界书 |
| SQLite 并发和跨平台迁移 | 本轮通过 | 并发首次打开、既有导入并发测试通过；精确 LF/CRLF 白名单且拒绝篡改；真机升级前后 20 张表一致 |
| Windows 原生写作流程 | 本轮通过（真实模型） | 4948da7 构建 13/13 项真实 Tauri IPC 检查；LLM 为真实 glm-5.3-flash（coding v4 端点，SSE 透传），33 次调用全 200（25 流式+8 非流式），tool_mode=native 一次通过；L1 磁盘 connections.json 仅 SecretRef 引用、无明文 key，杀进程重启可读（详见下节） |
| Windows/Android 调试候选 | 本轮产出并验证 | Windows 原生运行；Android APK 签名校验及保留数据覆盖安装通过，仍是调试签名 |
| 正式分发安装包 | 进行中 | 依赖已恢复（残留 vite/esbuild 进程占用已清理，node_modules 重装、npm ci 与前端构建通过）；计划版本 0.1.2 + tag v0.1.2 走 GitHub Actions release.yml 出 Windows msi+nsis、Android arm64 签名 APK 与 GitHub Release；构建结果未发生，不预写成功 |
| 当前候选完整真机写作、第三方插件 | 已闭合（本轮通过） | Windows 原生真实 glm-5.3-flash 写作（上列行）；Android 真机 9/9 环节：GUI 表单配置连接（测试连接 4s 连通）、SecretRef 落盘、续写 91s、采纳记账、对手戏 4 段编排 10.5 分钟、杀进程冷重启全持久，正文真实生成（Campaign 56→603 字），全程未用 IPC 建连接；第三方插件两条通道：1B 真实 TavernHelper 卡 5/5 远程脚本（含 MVU bundle）执行成功，1A manifest 插件 4 个真实缺陷修复并复验。边界与证据见下节 |
| 当前提交的远端 CI | 上一提交已推送并验证；本地 0.1.2 改动未推送 | main 4948da7 已推送 Gitea+GitHub 双远端；GitHub Actions release.yml workflow_dispatch 调试构建于该提交全绿（run 33980646148：windows job 27m29s、android job 20m，产物 20.8MB/12.4MB，4 个 ANDROID_* 签名 Secret 证明有效）；Gitea ci-gates run #33 同一提交全部 job 失败，根因是 runner 工作区缓存 "Unable to pull refs/heads/v4: non-fast-forward update"（jd-linux-1 runner 环境问题，非代码问题，GitHub 同提交全绿佐证）。远端构建按既定决策以 GitHub 为准；Gitea runner 缓存需服务器侧清理，本轮不处理 |

整体状态：**真实模型写作、第三方插件样本与远端 CI 调试构建证据已闭合；插件修复 7 文件后的本地完整 11 步门禁待重跑（结果见占位）；正式分发安装包进行中**。命令、证据、候选哈希及失败过程见 [`release-closure-2026-09-06.md`](release-closure-2026-09-06.md)。不得将此状态改写成“整个项目已正式发布”。

停止位置：依赖已恢复——此前 `npm ci` 失败的根因是残留 vite/esbuild 进程占用 `lightningcss` 原生模块，进程清理后 `frontend/node_modules` 重装、`npm ci` 与前端生产构建均通过。写作验收所用 APK 为 `bb39b43` 代码（与 HEAD 4948da7 同代码，仅差 docs）；本轮后续插件修复（7 个文件）未含在该 APK 内，插件修复不触及写作/连接/存储路径，由确定性门禁覆盖。插件修复后的完整 11 步门禁重跑结果：**通过（2026-09-06 13:02，`scripts/verify-release.ps1` 退出码 0）：secret 扫描、Pester 四套 198 项、fmt、严格 Clippy、Rust 1980 通过/0 失败/33 忽略、前端 504+119+9+28+2=662 项、生产构建全部通过；日志在 `artifacts/release-gate-2026-09-06/gate-run.log`**。历史 PASS 不代表当前工作区改动已验收。

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
  4. 事件广播 postMessage 传 Vue reactive 对象抛 DataCloneError（一轮 1003 条）——已修（payload 统一 JSON 往返解克隆）；复验 events=20、0 DataCloneError。
- 复验硬证据：prompt hook 标记 `[plugin-ok]` 进入生成轮全部 4 类上游请求（主生成/质量修订/摘要/后处理），hookCalls=changes=15；审计导出 integrity valid。修复共 7 个文件（Rust 2 + 前端 5），新增单测 5 个（前端 504/504 绿）。

### 边界（不得夸大）

- 本轮证明真实第三方样本经两条通道加载并工作（脚本执行/变量/prompt hook/审计/事件），不等于 ST 99 事件全集、完整 TavernHelper 语义、密码学审计或插件沙箱攻击面评估。
- 如实记录：裸 IPC `start_writing` 不经过前端 hook 接线属设计行为（fail-open）；采纳后的记账后处理调用无 prompt hook（路径差异，未深究）；小票组件不自动刷新为已知 UI 细节。
- Gate 6 维持“关闭非 PASS”不变；不宣称任何覆盖率指标；历史 PASS 不代表后续改动已验收。

### 远端 CI

- main（4948da7）已推送 Gitea + GitHub 双远端。
- GitHub Actions release.yml workflow_dispatch 调试构建于 4948da7 全绿：run 33980646148（windows job 27m29s、android job 20m；产物 windows-artifacts 20.8MB / android-artifacts 12.4MB；android job 证明 4 个 ANDROID_* 签名 Secret 有效）。URL：https://github.com/shis23/story/actions/runs/33980646148
- Gitea ci-gates run #33 同一提交全部 job 失败，根因是 runner 工作区缓存 "Unable to pull refs/heads/v4: non-fast-forward update"（jd-linux-1 runner 环境问题，非代码问题；GitHub 同提交全绿佐证）。按既定决策远端构建以 GitHub 为准；Gitea runner 缓存需服务器侧清理，本轮不处理。

### 正式分发安装包（计划，未完成）

- 依赖已恢复：残留 vite/esbuild 进程占用清理后，node_modules 重装、npm ci、前端 build 通过。
- 计划：版本 0.1.2 + tag v0.1.2 走 GitHub Actions release.yml，出 Windows msi+nsis、Android arm64 签名 APK 与 GitHub Release。此处只写计划与依据，构建结果未发生，不预写成功；实际结果发生后补记。

### 证据目录

- `artifacts/realmodel-2026-09-06/windows/`（gitignored）
- `artifacts/realmodel-2026-09-06/android/`
- `artifacts/plugin-acceptance-2026-09-06/`（summary.md + rerun-1a/ + rerun-1b/）

## 已有证据

- 2026-09-06 真实模型/插件/远端 CI 闭合记录：见上节「2026-09-06 真实模型与插件闭合」；明细另见 [`release-closure-2026-09-06.md`](release-closure-2026-09-06.md) 追加章节。

- 2026-09-05 桌面架构修复记录：删除未采纳稿件、已采纳前缀保护、末尾分支、v3 快照、模式无关 Editor 修订和隔离 Card Studio Store 已实现。历史记录为 Rust 1974 通过、33 忽略及 13 项真实 Tauri IPC 检查；模型为本机协议夹具。
- `android-topbar-acceptance-2026-09-05.md`：真机安全区、顶栏、键盘、面板切换及无清数据覆盖安装。它不是凭据或真实模型写作验收。
- `frontend-polish-2026-09-05.md`：响应式布局、动效、侧栏折叠和 Windows 原生检查。
- `theme-palettes-2026-09-05.md`：四套配色及浅/深模式；历史记录 655 项前端自动化通过。
- Gate 6：Canary3/Coverage12/TextFallback3/Stability30 的历史真实模型证据已封存；Full100 未完成，2026-08-31 已决议关闭但不是 PASS，不再作为本次前置续跑。

## 数据与权限

验收使用隔离数据和合成卡，不清空用户数据库，不绕过迁移校验，不用明文凭据替代系统安全存储。新发布证书、付费模型的大规模消耗、对外推送和发布需要相应授权及可用条件。

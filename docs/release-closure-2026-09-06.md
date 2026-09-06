# 0.1.1 收尾验收

执行时间：2026-09-05 至 2026-09-06，Asia/Shanghai。

结论：本地 11 步确定性门禁通过；四类修复完成回归和限定原生验证。正式发布、真实模型和完整第三方插件门禁仍未全部闭合。

## 修复内容

1. 系统凭据直接使用显式初始化的 `keyring_core`，移除在 Android/iOS 独立拒绝默认存储的包装层。新密钥不得以明文降级保存。
2. ST 导出使用当前 Campaign 世界书；PNG 保留本局世界书，附带 lorebook 合并已获取知识并避开条目 ID 冲突。底层转换的 `None` 表示不覆盖，显式空世界书表示清空。
3. SQLite 打开前设置 busy timeout；首次启用 WAL 遇到 BUSY 时做有界重试。已发布 V001-V008 只兼容精确 LF/CRLF 校验和，迁移账本不重写，修改过的 SQL 仍失败。未来 Git 检出固定 SQL 为 LF；反向导出使用相同校验政策。
4. 统一发布门禁纳入发布脚本、组件、CSP、移动布局和应用壳套件，共 11 步。UI smoke 从任意工作目录定位配置，并直接持有/结束自己的 Vite 进程。

## 确定性门禁

命令：

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\verify-release.ps1
```

本轮环境：Windows PowerShell、Pester 3.4.0、Python/PyYAML 6.0.2、Node 24、锁定的前端依赖及 Chromium。设置 `CARGO_BUILD_JOBS=2` 限制编译并发；没有设置 `RUST_TEST_THREADS=1`。

| 检查 | 结果 |
| --- | --- |
| Secret scan / fmt / Clippy `-D warnings` | 通过 |
| Rust workspace | 1980 通过，0 失败，33 忽略 |
| 前端 Node / Vitest | 499 / 119 通过 |
| Chromium CSP / 移动布局动效配色 / 应用壳 | 9 / 28 / 2 通过 |
| Pester 四套发布合同 | 51 + 14 + 34 + 99 = 198 通过，无跳过 |
| 前端生产构建 | 通过，保留包体和混合静态/动态导入警告 |

不声称覆盖率达到某个百分比，也不将需要真实模型或平台条件的 33 个忽略用例计为通过。UI smoke 另从仓库根目录直接执行通过，专用端口 41763 在结束后无监听。

## 原生专项

- Windows 实际系统凭据写/读/删测试通过：`cargo test -p storyforge-infra-util system_keyring_write_read_delete_roundtrip -- --ignored --nocapture`。
- Windows 调试候选以独立 AppData、SQLite 和 WebView 运行。13 项真实 Tauri IPC 检查通过：三种生成模式、采纳、未采纳稿件删除、已采纳历史保护、末尾分支、v3 快照恢复以及恢复后的继续生成。LLM 为仅监听本机的协议夹具，没有付费请求。IPC 驱动中的前端 prompt-hook 超时警告仍有记录，不能据此宣称真实插件主流程通过。
- Android 设备为 Redmi 23117RK66C，Android 16 / API 36。新旧调试 APK 证书相同，`adb install -r` 成功；未卸载、未清库。
- 真机六项检查通过：既有 Campaign 可读、Keystore 保存和 DTO 脱敏、重启后解密、删除后重新挂回旧 SecretRef 仍无法解密、旧明文配置迁移并重启读取、ST PNG 导出和原生后端重新导入保留两条世界书。
- 升级前后 SQLite 快照逐表对比：20 张表的行内容完全一致，包括 8 条迁移账本；两份数据库 `integrity_check=ok`。原故事、角色、世界书和 authority 没有被验收清空或改写。
- 原生截图已检查。上述 IPC/专项不能替代真实文件选择器、真实模型、后台生成、断网恢复及第三方插件的完整验收。

## 候选身份

这些是基于 `1ca37a2` 加本轮修复构建的调试候选；修复随后提交为 `bb39b43`。它们不是干净发布输入或生产签名的声明。构建后的调整只涉及测试夹具和门禁/验收文档。

| 产物 | SHA-256 |
| --- | --- |
| Windows debug EXE | `1e1cf441e5e0fb7f53dde324c10382b3d8687ba80c3ca9b9e47251641fee63dc` |
| Android arm64 debug APK | `27a1184de03c7056614829835a0e5729c0741bcc0e3f5c551484db4c574ede12` |
| Android 调试证书 | `04ba8572f716899d3f32cc477d53fc3f46cfd602a1cb6ba00958eb882a39d4f1` |

## 失败与边界

- 初轮全量 Rust 测试暴露并发首次打开锁竞争；修复后新增竞争回归和既有并发导入测试通过。
- 新增测试曾因未初始化独立 SQLite runtime、反向导出夹具目标落入活动数据根而失败。修正测试夹具后重新执行完整门禁，没有跳过失败项。
- ADB 旧管道写法曾中断合成连接配置，触发应用写入保护；恢复了本轮验收前已确认的空连接配置，改用文件推送和读回校验后，完整凭据专项通过。没有真实密钥或故事数据库修复操作。
- 真机有一局旧 v2 导入数据缺失源角色卡。当前版本拒绝导出不完整 v3 快照；失败已保留，原数据未改写。新格式测试通过不代表这份历史缺失数据已修复。
- 真实模型与预算、第三方插件样本、当前候选正式分发安装验收，以及推送后的远端 CI 仍需闭合。未执行推送、打 tag 或发布。
- 修复提交后尝试了 Windows 正式发布流程，干净输入检查通过，但在 `npm ci` 阶段因 `lightningcss.win32-x64-msvc.node` 占用（EPERM）失败；没有到 Rust release 或安装器构建。依赖目录因此不完整，追加 Android 三模式写作检查因找不到 Playwright 而未启动，不能计为通过。
- 用户随后要求停止测试和构建。本轮调试 App、模型夹具和 Android 构建残留进程已关闭，没有重装依赖或继续构建。恢复开发时须先处理本地文件占用并重新安装锁定依赖；此前完整门禁结果仍是停止前的已执行证据，不代表目前依赖目录可直接运行。

## 本地证据

原始日志和合成夹具在忽略目录 `artifacts/release-closure-2026-09-05/`，目录日期为任务开始日。

- `gate-verified.log`：完整 11 步成功；`full-gate*.log` 保留此前失败。
- `windows-build.log`、`android-build.log`：原生调试构建日志。
- `windows-release-build.log`：正式发布流程在依赖安装阶段失败的记录。
- `windows/ipc-verification.json`、`windows/fixed-story-native.png`。
- `android/verification.json`、`android/restored-story.png`、`android/legacy-source-failure.json`。
- `android-before.tar`、`android-database-comparison.json`：升级前备份与逐表比较。

这些本地产物不强制加入 Git；公开验收边界和复验命令由本文保留。

## 2026-09-06 晚间续：真实模型/插件/远端 CI 闭合与插件缺陷修复

本节为追加记录，不改写上文。执行时间：2026-09-06 晚，Asia/Shanghai。上文所述「真实模型与预算、第三方插件样本、远端 CI 未闭合」以及「npm ci EPERM、依赖目录不完整」两项停止状态在本节闭合或解除，均按保守口径记录。

### 真实模型写作

- Windows 原生候选（HEAD 4948da7 构建，EXE SHA-256 前缀 `3A2867F8…CB84`）：13/13 项真实 Tauri IPC 检查全部通过。LLM 为真实 glm-5.3-flash（open.bigmodel.cn coding v4 端点，经本机记录代理，SSE 透传）：33 次调用全 200（25 流式 + 8 非流式），流式首 token 中位 2207ms，单调用中位 70.9s，token 下界 144,854（其中 reasoning 85,916）。tool_mode=native 一次通过。L1 磁盘 `connections.json` 仅 `storyforge-secret:v1` 引用、无明文 key；杀进程重启后 `get_active_connection` 可读。
- Android 真机（Redmi 23117RK66C，Android 16，APK SHA-256 `27a1184d…4ede12` 与候选一致）：9/9 环节通过——GUI 表单配置连接（测试连接 4s「连通成功 glm-5.3-flash」）、SecretRef 落盘校验、续写生成 91s、采纳记账（3 项）、对手戏第二轮 10.5 分钟（4 段编排）+ 采纳（4 项）、杀进程冷重启后连接/正文/会话全持久。正文真实生成（Campaign 56→603 字）。全程未用 IPC 建连接。
- 口径说明：Android 验收所用 APK 为 `bb39b43` 代码（与 4948da7 同代码，仅差 docs）；本轮后续插件修复（7 文件）未含在该 APK 内。插件修复不触及写作/连接/存储路径，由确定性门禁覆盖；修复后的完整 11 步门禁重跑结果：**通过（2026-09-06 13:02，`scripts/verify-release.ps1` 退出码 0）：secret 扫描、Pester 四套 198 项、fmt、严格 Clippy、Rust 1980 通过/0 失败/33 忽略、前端 504+119+9+28+2=662 项、生产构建全部通过；日志在 `artifacts/release-gate-2026-09-06/gate-run.log`**。

### 第三方插件（两条通道）

- 通道 1B（卡内真实 TavernHelper 脚本）：真实卡「命定之诗与黄昏之歌 v4.1」GUI 导入（原生文件对话框 Win32 直驱），5/5 远程脚本（含 MagVarUpdate bundle.js via jsdelivr）全部执行成功，MVU 变量初始化并在真实 LLM 轮后更新，开场轮采纳。PASS。
- 通道 1A（manifest 插件）：首轮验收发现 4 个真实缺陷并全部修复+复验，根因各一条：
  1. `InstalledPluginDto` 缺 `entry_html` 字段，前端拿不到入口页，iframe 永不启动——已修；GUI 安装的插件 iframe 真实启动。
  2. DOMPurify 默认剥除 `<script>`，插件 HTML 失去脚本载体——已修（`ADD_TAGS script`）；安全边界在 sandbox iframe + 权限门控桥，注释已论证。
  3. 审计脱敏非幂等，重复脱敏破坏 ring buffer 一致性检查，fail-closed 在 1 条——已修；6/6 入库 integrity valid。
  4. 事件广播 postMessage 直传 Vue reactive 代理对象，结构化克隆抛 DataCloneError（一轮 1003 条）——已修（payload 统一 JSON 往返解克隆）；复验 events=20、0 DataCloneError。
- 复验硬证据：prompt hook 标记 `[plugin-ok]` 进入生成轮全部 4 类上游请求（主生成/质量修订/摘要/后处理），hookCalls=changes=15；审计导出 integrity valid。修复共 7 个文件（Rust 2 + 前端 5），新增单测 5 个（前端 504/504 绿）。

### 过程如实记录

- Android 验收中途 ADB 一度报 `unauthorized`（真机调试授权态失效）导致连接中断，在真机上重新确认 USB 调试授权后恢复，已完成环节的落盘证据未受影响。
- 真机验收中出现测试卡「识别失败已按单角色处理」横幅：应用对该卡降级为单角色继续流程，未静默失败；该横幅如实记录，不代表角色卡识别全场景通过。
- 依赖恢复：上文 `npm ci` EPERM 的根因是残留 vite/esbuild 进程占用 `lightningcss` 原生模块；进程清理后 `frontend/node_modules` 重装、`npm ci` 与前端生产构建均通过。

### 边界（不得夸大）

- 本轮证明真实第三方样本经两条通道加载并工作（脚本执行/变量/prompt hook/审计/事件），不等于 ST 99 事件全集、完整 TavernHelper 语义、密码学审计或插件沙箱攻击面评估。
- 裸 IPC `start_writing` 不经过前端 hook 接线属设计行为（fail-open）；采纳后的记账后处理调用无 prompt hook（路径差异，未深究）；小票组件不自动刷新为已知 UI 细节。
- Gate 6 维持「关闭非 PASS」不变；不宣称任何覆盖率指标；历史 PASS 不代表后续改动已验收。

### 远端 CI

- main（4948da7）已推送 Gitea + GitHub 双远端。
- GitHub Actions release.yml workflow_dispatch 调试构建于 4948da7 全绿：run 33980646148（windows job 27m29s、android job 20m；产物 windows-artifacts 20.8MB / android-artifacts 12.4MB；android job 证明 4 个 ANDROID_* 签名 Secret 有效）。URL：https://github.com/shis23/story/actions/runs/33980646148
- Gitea ci-gates run #33 同一提交全部 job 失败，根因是 runner 工作区缓存 "Unable to pull refs/heads/v4: non-fast-forward update"（jd-linux-1 runner 环境问题，非代码问题；GitHub 同提交全绿佐证）。按既定决策远端构建以 GitHub 为准；Gitea runner 缓存需服务器侧清理，本轮不处理。

### 正式分发安装包（计划，未完成）

- 计划：版本 0.1.2 + tag v0.1.2 走 GitHub Actions release.yml，出 Windows msi+nsis、Android arm64 签名 APK 与 GitHub Release。此处只写计划与依据，构建结果未发生，不预写成功；实际结果发生后补记。

### 门禁重跑

- 插件修复 7 文件后的完整 11 步确定性门禁重跑结果：**通过（2026-09-06 13:02，`scripts/verify-release.ps1` 退出码 0）：secret 扫描、Pester 四套 198 项、fmt、严格 Clippy、Rust 1980 通过/0 失败/33 忽略、前端 504+119+9+28+2=662 项、生产构建全部通过；日志在 `artifacts/release-gate-2026-09-06/gate-run.log`**。

### 证据目录

- `artifacts/realmodel-2026-09-06/windows/`（gitignored）
- `artifacts/realmodel-2026-09-06/android/`
- `artifacts/plugin-acceptance-2026-09-06/`（summary.md + rerun-1a/ + rerun-1b/）

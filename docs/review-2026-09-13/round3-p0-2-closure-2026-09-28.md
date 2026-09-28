# Round 3：P0-2（M-01）子帧 IPC 越权——运行时 PoC、修复与复验收口

- **日期**：2026-09-28（接续 `round2/FULL-REVIEW-REPORT-v2.md` §8.1「唯一仍开放的 P0」）
- **基线**：`a891be3`（2026-09-13 审查轮已落库）之上的工作树改动（本轮修复）
- **执行环境**：本机 Windows 桌面会话（WebView2 `Edg/153.0.4234.48`，CDP 远程调试口 9223），
  debug 构建 `target/debug/storyforge.exe`，前端 dist 为门禁刚构建的产物
- **沙箱隔离**：子进程 `APPDATA`/`LOCALAPPDATA` 指向 `%TEMP%\storyforge-p02\` 临时目录，
  全程未触碰用户真实应用数据；只调用只读命令（`list_conversations`、`list_agent_profile_configs`、
  `card_shell_register_doc`）

## 1 运行时 PoC（修复前）——R5 §3.3 的推断全部坐实

方法：真实启动应用 → CDP 注入 iframe 指向**经 `card_shell_register_doc` 正规注册**的 shell 文档
（与 CardShellHost/PluginHost 前端同一条路径）→ 在该子帧上下文执行 R5 §3.3 第 3-6 步。

| # | 检查（R5 §3.3 步骤） | 结果 | 判定 |
|---|---|---|---|
| 1 | 主帧 URL | `http://tauri.localhost/` | 主帧正常 |
| 2 | **子帧 URL/origin**（决定 ACL 分支） | `http://storyforge-shell.localhost/<64位token>` | **local 坐实**（R5 推断→实测） |
| 3 | IPC 引导是否进子帧 | `__TAURI_INTERNALS__: object`、`.invoke: function`、`window.ipc: object`、`isTauri: boolean` | **wry 全帧注入坐实** |
| 4 | **决定性调用** `invoke('list_conversations')` | **`SUCCESS: []`（约 1 秒返回）** | **越权调用成功** |
| 5 | 升级探测：零前端入口命令 `list_agent_profile_configs` | **`SUCCESS`，带出内置 Agent 配置完整数据**（读外泄证实） | 攻击面确认 |
| 6 | 负对照（主帧同命令） | `SUCCESS: []` | 命令本身可用，差异来自帧 |
| 7 | 错误可见性对照（未知命令，主帧） | `Command definitely_not_a_command_xyz not found` | 成功/拒绝可区分，第 4 步是真成功 |

证据文件：`artifacts/p02-poc/poc-evidence-before-fix.json`（注入帧）；应用自带的 MVU 壳帧另见
诊断记录（§3 附注）。

**结论：P0-2 从「源码级机制链 + 高置信推断」升级为「运行时实测确认」——任意
`storyforge-shell` 源子帧可直接调用全部 175 个注册命令。**

## 2 修复：`SUBFRAME_IPC_GUARD`（init 脚本尾部的子帧毒化）

上游调研（2026-09-28，随本轮完成）：wry 0.55.1→0.57.0 **没有也不会**修——Windows 子帧注入是
WebView2 `AddScriptToExecuteOnDocumentCreated` API 的既定行为，wry 文档明说"until Webview2
implements a proper API"；插件生态曾主动要求全帧注入（wry#1531，2025-04）。Tauri 官方
CVE-2024-35222 的修复也只覆盖非同源 iframe。**结论：只能应用层修。**

实现（`crates/tauri-app/src/shell_doc_protocol.rs` + `lib.rs` 接线）：
`tauri::Builder::append_invoke_initialization_script` 把守卫追加到 **invoke 初始化脚本本体末尾**
（脚本顺序天然确定：紧跟 IPC 引导之后、其余 tauri init 脚本之前），子帧（`window.self !==
window.top`）内四层失效：

1. **冻结 `__TAURI_INTERNALS__`**——window 上的属性本身不可配置不可删，但冻结值后，
   metadata / core.js（`invoke`、`convertFileSrc`）/ 事件与插件 init 脚本全部无法附着；
2. `sendIpcMessage` 的 **fetch 分支**因缺 `convertFileSrc` 在构造 URL 时即 TypeError，**请求发不出**；
   postMessage 回退分支被闭包私有标志挡住，不可达；
3. **封传输通道**：`window.fetch` 包装——`ipc:` / `http(s)://ipc.localhost` 目标直接拒绝，其余 URL
   原样放行（壳文档资产/模块 fetch 不受影响，复验第 4 步证明无过杀）；`window.ipc` 删除并封死
   （Android 无 fetch 分支、直走 postMessage，同样被覆盖）；
4. **key 保密兜底**：invoke key 只存在于冻结对象的闭包里，tauri 刻意声明在暴露函数之外
   （`toString` 不泄漏），页面代码无从读取；手工裸载荷（`chrome.webview.postMessage`）过不了
   `on_message` 的 key 校验（不匹配即静默丢弃）。

主帧（`self === window.top`）零改动；macOS/Linux 本就不向子帧注入 init 脚本，守卫自然不运行。

**与 R5 §7.3 修法清单的对应**：即其 (c)「命令层主帧凭证」的实现——凭证就是 invoke key，
本修复让子帧拿不到凭证。

## 3 修复后复验（同环境同方法）——全部通过

| # | 检查 | 修复后结果 | 判定 |
|---|---|---|---|
| 1 | 子帧 internals 状态 | `frozen: true`，`invoke: undefined`，`convertFileSrc: undefined` | ✓ 引导装不上 |
| 2 | 子帧 `invoke('list_conversations')` | **BLOCKED**：`invoke is not a function` | ✓ 越权调用死 |
| 3 | 残存 `postMessage` 闭包直调（最后手段） | **DEAD**：`convertFileSrc is not a function`，未发出任何请求 | ✓ 深度防护 |
| 4 | 裸 `fetch('http://ipc.localhost/...')` | **FETCH-BLOCKED**：`storyforge: subframe IPC is disabled (P0-2)` | ✓ 传输封堵 |
| 5 | 非 ipc fetch 放行（防过杀） | `PASSTHROUGH-OK`（404=一次性文档已被消费，符合设计） | ✓ 无过杀 |
| 6 | **主帧负对照** `invoke('list_conversations')` | **SUCCESS `[]`**；internals 未冻结、`invoke: function` | ✓ 应用功能完好 |

证据文件：`artifacts/p02-poc/poc-evidence-after-fix.json`。

### 附注：应用自带 MVU 壳帧的挂起现象（非缺陷，如实记录）

PoC 过程中发现应用自身的 MVU 运行时壳帧调用 `invoke` 会**挂起**（12s+ 不结算）：该文档自身
禁用了 fetch（"MVU: fetch disabled"），走 `window.ipc.postMessage` 回退——消息可能已到达 Rust 侧
但响应经主帧 `eval` 路由回不到子帧的 callback 注册表，promise 永不结算。**这一现象不改变第 1 节
结论**（朴素帧 fetch 路径 1 秒成功才是决定性证据），但它说明：postMessage 传输下子帧调用
"不结算"≠"不执行"——写命令可能已生效。修复后该路径整体死亡（§3 第 3 步），此不确定性一并消除。

## 4 守卫测试与门禁

- `shell_doc_protocol.rs` 新增 2 条守卫测试：`subframe_guard_only_acts_in_subframes_and_blocks_both_transports`
  （静态断言：子帧限定、freeze、ipc 目标 fetch 拦截、放行通道、window.ipc 封死、纯 ASCII、不引用
  invoke key）与 `subframe_guard_is_wired_into_the_builder`（防重构摘除）；
- `acl_manifest_absence_is_a_known_risk` 告警文案更新：ACL 粒度区分不了同源子帧，运行时缓解即本守卫；
- 全量 11 步门禁于本报告落地后复跑（见 RELEASE-STATUS「停止位置」）。

## 5 R12-R3 顺带收口（同一批）

`commands/writing.rs` 的 JSON 直写路径孪生门禁 `should_block_source_knowledge_propagation`
补齐三处 fail-open 的 warn（与 `production_postprocess.rs` SQLite 路径的
`reason` 字符串逐字一致，便于跨路径聚合），零行为变化；`cargo test -p storyforge --lib propagation`
3/3 通过。

## 6 诚实边界（未验证部分）

1. **release 构建未复验**：守卫是无 cfg 差异的 JS 注入，debug/release 行为应当一致，但本轮 PoC
   全部跑在 debug 构建；release 构建 CDP 工具链（devtools 特性）未验证。
2. **macOS/Linux/Android 真机未跑**：macOS/Linux 不向子帧注入（守卫不运行、维持原状）；Android
   理论被 window.ipc 封死覆盖，未真机验证。
3. **护栏强度**：本修复是脚本层（非 Rust 传输层）拒绝。绕过需要 (a) WebView2/Chromium 自身漏洞，
   (b) tauri 升级改变脚本组装顺序或 key 存放方式，或 (c) 应用引入新的本地源 iframe 通道。守卫测试
   `subframe_guard_is_wired_into_the_builder` 只防"被摘除"，不防 tauri 升级引入的形态漂移——
   **升级 tauri 时应重跑本报告的 PoC**（脚本配方：`%TEMP%` 下按 §1 方法重建，或参照
   `artifacts/p02-poc/*.json` 的步骤记录）。
4. ACL manifest 仍不存在（有意）：补 manifest 不能区分同源子帧，反而会因 fail-closed 拒掉主窗口
   全部命令，需逐条白名单后才有净收益——维持现状并继续由守卫测试记录。

## 7 附带：Tauri 2.11.5 → 2.11.6（GHSA-w28w-mhc8-qvjv，随本轮处理）

上游调研发现 2026-09-26 新公告 **GHSA-w28w-mhc8-qvjv**（High，CVSS 7.6，"Improper Tauri IPC
Access Control for Fetch Command"：`plugin:__TAURI_CHANNEL__|fetch` 被 ACL 豁免 + channel 队列
顺序 u32 ID 可预测 ⇒ 跨 WebView 窃取 channel 载荷），影响 `tauri >=2.0.0, <=2.11.5`——本仓库
原锁定的 2.11.5 正好在范围内。已执行 `cargo update -p tauri --precise 2.11.6`（锁文件差异恰好
一处版本号；wry 保持 0.55.1；`ipc-protocol.js` 与 `manager/webview.rs` 脚本组装逐字节不变）。
按 §6.3 自己定的规则，升级后重跑了 PoC 复验：结果与 §3 完全一致
（`artifacts/p02-poc/poc-evidence-after-fix-tauri2116.json`）。另确认 GHSA-7gmj-67g7-phm9
（origin 混淆，<=2.11.0）不在影响范围。CVE-2024-35222（iframe 绕过 origin 检查）的历史修复
已在 2.11.x 内，其 Windows 同源残余路径即本报告 §1 处理的对象。

## 8 状态

**P0-2 / M-01：关闭（运行时 PoC 实锤 → 修复 → 同法复验全部通过）。**
R5 §3.3 要求的"PoC 实测后再定方案"已完成；二轮报告 §8.1 的唯一开放 P0 就此清零。

# test-card 壳路径证据清单

> 金标：`test-card.png`（命定之诗与黄昏之歌 v4.1）  
> 复跑：`scripts/run-card-shell-evidence.ps1`  
> 计划真相源：`docs/workstreams/CAMPAIGN-WORLDINFO-AND-CARD-SHELL-PLAN.md`  
> 文档更新：2026-07-23（**暂停改代码**，只记现状）

## 自动断言（CI/本地）

| # | 检查 | 命令 / 位置 |
| --- | --- | --- |
| 1 | 提取器：home/custom_start/status + ≥5 TH | `cargo test -p storyforge-domain card_shell`（包名以仓库为准） |
| 2 | allowlist 拒非白名单 host | `cargo test` 中 `card_shell_cache` |
| 3 | display `body.load` 分流 | `node --test frontend/tests/card-shell-display.test.mjs` |
| 4 | TH 顺序 / 按钮解析 | `node --test frontend/tests/tavern-helper-scripts.test.mjs` |
| 5 | 变量出站 campaign/instance | `node --test frontend/tests/shell-variable-outbox.test.mjs` |
| 6 | 世界书活动隔离 | `campaign_store` 世界书相关测 |
| 7 | plugin-bridge ST 面 | `node --test frontend/tests/plugin-bridge.test.mjs`（71 pass @ 2026-07-23） |

## 手工 UI 证据（Tauri，用户会话 2026-07-22～23）

| 步骤 | 期望 | 实测 |
| --- | --- | --- |
| 导入 test-card → 开活动 | 不白屏不崩 | 通过（世界书 IPC 截断后） |
| TH 状态条 | 6 脚本顺序 + 按钮 | **6/6 全部完成** |
| 状态栏壳 | status 远程 UI | **有** Tab（任务/信息/持有物/命定/新闻/地图） |
| 开场壳 | home「制作团队 / Destined Poetry」全页 | **未通过**：界面像 status 信息/残缺态，不是制作团队金标 |
| 消息内壳 | display 分流挂载 | 有「已加载 消息首页壳」；内容未金标验收 |
| 变量出站 | setvar → campaign/instance | 代码有；与本卡 MVU 同源未封 |
| 非 allowlist | 明确错误 | 路径存在；未单独截图 |

## 代码锚点

| 能力 | 路径 |
| --- | --- |
| 壳提取 | `crates/domain/src/card_shell.rs` |
| 宿主缓存 | `crates/tauri-app/src/card_shell_cache.rs` |
| 可见壳 | `frontend/src/components/CardShellHost.vue`（`8e97af8` 起注入 `generateBridgeScript`） |
| TH | `frontend/src/components/TavernHelperRuntime.vue` |
| ST 权威面 | `frontend/src/plugin-bridge.js` |
| 写作挂载 | `frontend/src/AppV2.vue` `#shell` |
| display 分流 | `frontend/src/components-v2/st/ShellAwareContent.vue` |

## 库存 URL

见 `docs/workstreams/test-card-shell-inventory.json`。

## 明确未完成（勿写「已完整」）

1. 开场 home 金标 UI（制作团队页）  
2. 开局变量灌入壳 `getVariables({type:'character'})` 验收  
3. 金标截图包入库  
4. 壳与 PluginHost 事件/消息镜像完全同权（非本卡必达，但 home 依赖 top/TH 时相关）

## 过程备注

- 手搓壳内 ST free function 易与已有 `plugin-bridge` 分叉；后续应以 bridge 为单一来源。  
- 用户确认视觉正确后勿擅自回滚到旧 shim。  
- 重启可能导致「同提交看起来又坏」；对比应用提交哈希 + 干净 `bash dev.sh`。

## 2026-07-23 真实桌面 UI 续验

- Tauri 桌面端 Campaign `命定之诗 UI验收 2026-07-23` 已实际完成一轮 `cpa` 多 Agent 写作；LLM 日志显示真实耗时、token 与 cache 命中。
- 已实际点击变体的“采纳”，页面按钮状态变为“已采纳”，当前会话正文为 1,436 字。
- 开场 home 与 status 壳同时挂载时的 inline module 串线已修复：每个宿主使用独立 module id，非拥有宿主静默忽略请求；home 可见 `命定之诗与黄昏之歌`、`Destined Poetry & Twilight Song` 与制作团队内容。
- 概览页“继续写作”现在会恢复该 Campaign 最近更新的会话，而非进入空白写作页；已在 Tauri 中复验。
- 首页“环境检查”不再永久显示“加载中”。它现在如实显示当前桥接能力：TavernHelper 版本未知、EJS 未检测到、MVU 异常/超时；这些项**不是通过**，也没有被伪造为可用。
- 前端全套：`npm run test:all` 337 node tests + 35 Vitest tests 通过；`npm run build` 通过（仅现有 chunk-size/dynamic-import 警告）。

### 仍未完成

要让 home 环境检查全绿，仍需实现真实的 TavernHelper 版本/世界书兼容层、EJS 模板引擎与可暴露给 card shell 的 MVU 运行时；在这些能力实际存在前，当前 UI 保持明确的不可用状态。

## 2026-07-23 最终桌面复验（取代上方“仍未完成”判断）

本节是在重新执行 `npm run build`、`cargo build -p storyforge` 后，用
`target/debug/storyforge.exe` 的真实 WebView2 窗口完成；不是浏览器 mock 或
单元测试替代品。

| 验收项 | 真实证据 |
| --- | --- |
| 环境检查 | 从活动概览点击“继续写作”后，首页壳自动从 EnvCheck 进入 DLC 页。原卡仅在 TavernHelper ≥ 4.3.17、EJS 已存在且启用、`waitGlobalInitialized('Mvu')` 成功时才会发出 `next`；因此三项均为真实可用。 |
| 首页与布局 | 消息内首页壳实际显示标题 `命定之诗与黄昏之歌`、英文副标题、制作团队、英灵殿、世界背景，并可滚动进入 DLC 与核心选择页。 |
| 消息内世界书 | 修复 `ShellAwareContent` 未传入活动 Campaign ID 后，DLC“角色”页从错误空态恢复为真实 30 个条目（如埃尔薇拉、爱丽丝、安娜斯塔西娅等）。 |
| DLC 写回 | UI 选择“埃尔薇拉”→“已启用”切为“已禁用”→“下一步/保存中”，活动书 `st_id=717622` 的 `disabled` 变为 `true`；再经相同 UI 路径恢复为 `false`。两次落盘均保留原 `Selective` 路由、6 个关键词和 2477 字正文。 |
| 核心选择 | DLC 保存后实际进入“核心选择”；“特别推荐 / 中杯 / 这是什么杯”Tab 与推荐卡（妲丽安核心、null核心、类脑娘）可从活动世界书和远端分类数据加载。 |
| 状态栏与 TH | 状态栏 6 个 Tab 均可见；实际点击“任务”显示“进行中 0”，点击“地图”显示“高清地图 / 超清地图”。TavernHelper 状态条为 6/6 `ok`。 |
| 模型、Agent 与日志 | 当前会话为真实 `cpa` 多 Agent 结果，1,436 字已采纳。调试“日志”面板实际显示 83 条记录及成功的 `LLM cpa 完成` 条目（耗时、输入/输出 token、cache 命中）；例如 27,945 ms、4122+2189 token、cache 命中 2944。重启后内存 Trace 为 0 是预期行为，持久日志仍可见。 |

本轮新增的回归保护：`ShellAwareContent` 从活动 Campaign store 绑定每个
消息内 `CardShellHost` 的 `campaign-id`；此前只有 AppV2 顶层壳有此 prop，
所以消息内壳的 `getCharWorldbookNames('current')` 返回空值并把 DLC 误显示为
“未找到可用角色”。

同一轮还将多条 DLC 开关的持久化改为顺序写入：Campaign 世界书是共享文件，
并发写入会造成相邻条目的更新丢失。每个 iframe 现在携带独立桥接会话令牌，
只有其所属宿主可以处理写回；同一 Campaign 的前端批次排队，后端读改写在同一
互斥锁内原子落盘。前端回归断言多宿主批次的最大持久化并发数为 1；后端并发开关
回归断言两个条目都会保留，避免相邻更新丢失。读取和差异计算也在同一 Campaign
队列内完成，因此后来的 shell 意图不会基于过期快照被当成无操作。若条目以
`Disabled` 路由保存，重新启用只会恢复由 ST 蓝/绿灯推导出的可用路由（蓝绿同时
启用则恢复 `Both`），无法恢复时显式报错而不伪装为已启用。

自动验证（本轮）：

- `npm run test:all`：343 Node tests + 36 Vitest tests 通过。
- `npm run build`：通过（仅已有 chunk-size/dynamic-import 警告）。
- `scripts/run-card-shell-evidence.ps1`：通过（提取、allowlist、活动书、display、TH、变量出站）。
- `cargo test -p storyforge --lib world_info`：6/6 通过（含并发持久化与 Disabled/Both 路由恢复）。

范围说明：这证明 `test-card.png` 实际依赖的首页、状态栏、TH、EJS、MVU 与
Campaign 世界书路径可运行；不宣称未被该卡使用的全部 SillyTavern 插件 API
已经 100% 等价。

## 2026-07-23 布局重排与本轮质量证据（最新）

本轮实现的目标布局为：**完整开场位于故事正文之前**；**状态栏紧随最后一条消息**；
**TavernHelper（TH）以紧凑侧栏呈现**。这是已落地的 UI 布局重排，不应再按旧的
“开场/状态/消息壳并列或位置未定”描述理解。

### 已通过的质量检查

- `npm run test:all`：**343 Node tests + 44 Vitest tests** 通过。
- `npm run build`：通过。
- `scripts/run-card-shell-evidence.ps1`：通过。
- `cargo build -p storyforge`：通过。
- 第二轮代码复审：通过。

### 真实桌面启动与视觉验收边界

- 新版本 Tauri 已真实成功启动，并已载入验收 Campaign。
- 但 Computer Use 在尝试操作该窗口时返回 **`failed to activate captured window`**，
  因而无法点击进入写作页。
- 所以，本轮**不能宣称完整视觉位置验收已完成**：上述“完整开场在故事前、状态跟随
  最后一条消息、TH 紧凑侧栏”的实际可交互位置，仍须在可交互桌面会话中复核。

这不影响构建、自动测试、证据脚本、Rust 构建及第二轮代码复审的通过结论；它只限制
真实桌面端的最终交互式视觉定位验收。

## 2026-07-23 截图反馈后的布局修正（最新）

- **序章只在开局阶段出现**：仅当尚未建立会话时显示；会话建立后不再保留序章区域。
- **序章高度自然增长**：通过受 session 保护的高度回传更新容器尺寸，内容增长时不会截断或
  留出固定空区。
- **状态栏按需运行**：默认折叠；用户点击展开后才挂载并运行，关闭时卸载。
- **无序章空白残留**：已有会话的写作页不会留下序章占位空白。

本轮质量结果：

- `npm run test:all`：**346 Node tests + 46 Vitest tests** 通过。
- `npm run build`：通过。
- 代码复审：通过。

## 2026-07-23 最终交互设计（最新）

- **序章显示授权收紧**：仅由“新建 Campaign”“选择角色”或“继续写作时尚未有会话”的流程
  显式授权显示；历史会话一律不显示序章。
- **状态栏入口与生命周期**：状态栏改为右下悬浮球；点击后以项目的 Headless UI `Dialog`
  全屏展开，关闭时卸载其 iframe。
- **Dialog 交互保证**：使用 portal 与背景遮罩，并支持 Esc/backdrop 关闭和焦点管理。

质量记录保持：`npm run test:all` **346 Node tests + 46 Vitest tests** 通过；
`npm run build` 通过；最终代码审查通过。

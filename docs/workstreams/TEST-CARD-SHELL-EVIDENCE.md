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

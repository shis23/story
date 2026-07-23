# Campaign 世界书 + 完整 Card Shell WebView

> 状态：**阶段性暂停改代码**（文档更新 2026-07-23）  
> 当前 HEAD 相关提交：`8e97af8`（壳复用 `plugin-bridge` ST 面）  
> 金标卡：仓库根 `test-card.png`（命定之诗与黄昏之歌 v4.1）

## 硬约束（不变）

- **不接受降级**：禁止用纯文本开场 / 禁网隐藏 iframe / 仅声明式状态栏冒充「完整壳」。
- 角色卡世界书 = 模板**只读**；Campaign 世界书 = 本局真相源**可写**；写作注入读活动书。
- 主应用 **不** 与卡脚本 same-origin；远程资源 **宿主代持**（allowlist + 缓存）。

## 分层（架构）

```text
角色卡世界书 = 模板只读
Campaign 世界书 = 本局真相源（可读写）
写作注入 = 活动书
CardShellHost = 可见 WebView + 宿主代持远程资源（开场/状态/消息壳）
MvuJsRuntime = 隐藏片段执行（并存，不替代壳）
plugin-bridge.js = 已有完整 ST/TavernHelper/SillyTavern 自由函数面（插件 iframe 同源方案）
```

## test-card 壳清单

| 用途 | URL / 形态 | 2026-07-23 实测 |
| --- | --- | --- |
| 首页 home | `…/FrontEnd-for-destined-journey@1.6.2/dist/home/index.html` | 有「制作团队 / Destined Poetry」文案；**当前挂载后界面不像该页**（见缺口） |
| 自定义开局 custom_start | `…/dist/custom_start/index.html` | 体量大；含基础信息等字段 |
| 状态栏 status | `…/dist/status/index.html` | 任务/信息/持有物/命定/新闻/地图 Tab **已能渲染** |
| TH ×6 | MagVarUpdate / data_schema / 自动化 / 预载 / 创意工坊 / 自动正则 | **6/6 全部完成** |

Display 正则将 `【首页】` / `<customized>` / `<StatusPlaceHolderImpl/>` 换成 `$('body').load(url)`；完整语义仍要求宿主代持 + 真执行。

---

## 阶段完成度（诚实）

| Phase | 内容 | 状态 |
| --- | --- | --- |
| 0 | 规格 + 壳清单 | **完成**（`docs/workstreams/*` + inventory JSON） |
| 1 | Campaign 世界书存储 / 开档拷贝 / 注入 / API | **完成** |
| 2 | 卡只读世界书 UI + 活动世界书 Tab | **完成** |
| 3 | 壳提取器 + allowlist 缓存后端 | **完成**（IPC 大 payload 有截断/延迟策略） |
| 4 | 可见 CardShellHost + 宿主 load | **部分完成**（能挂 URL、能跑模块；**开场视觉未达金标**） |
| 5 | 消息 display 分流 + 变量出站 | **部分完成**（`ShellAwareContent` / outbox 有；变量与 ST 变量袋是否与本卡完全一致未验收） |
| 6 | tavern_helper 顺序执行 | **完成（TH 6/6）** |
| 7 | Meta→活动书 + 证据 | **部分完成**（单元/脚本证据有；**金标 GUI 截图未闭环**） |

---

## 2026-07-22～23 实现要点（已落地代码）

### 世界书

- 旁路 `campaign_world_info/{id}.json`，开档拷贝，写作注入活动书。
- 卡 UI 只读；活动 Tab 读写；Meta world_info patch 写活动书。
- 大世界书列表 IPC 预览截断，展开再拉全文。

### Card Shell / TH 运行时

- `CardShellHost`：blob URL 加载（WebView2 上 srcdoc 不可靠）、宿主 `card_shell_fetch_url`、jQuery `.load` / `getScript` 桥。
- 壳 HTML 常为 head+body 无 `<html>`：包装成完整文档；inline `type=module` 改宿主取源 + **iframe 同源 blob 图**（父页 blob 跨源不可 import）。
- 预载：jQuery、Vue 3 global、Zod **v4** ESM（`prefault`/`loose`）、lodash 等。
- `TavernHelperRuntime`：postMessage 控制、模块同源 blob、全局 preamble（`z`/`Vue`/`$`/`YAML`/`SillyTavern`…）；**TH 6/6 ok**。
- **ST 面**：`frontend/src/plugin-bridge.js` 已有完整 `generateBridgeScript` + `createHostHandler`（`PluginHost` 在用）。`8e97af8` 起 `CardShellHost` **注入该 bridge**，避免在壳里无限手搓 free function。
- 变量选择器：`plugin-bridge` 已扩 `character` / `script` type（本卡 status 使用 `getVariables({type:'character'})`）。

### 关键提交簇（shell 相关，自新到旧摘录）

```
8e97af8 fix(card-shell): reuse plugin-bridge ST surface in shell iframes
deae7b6 … batch-shim status-shell ST free functions
e2fbb45 … expose ST getVariables API in shell iframes
ca229f6 … preload Zod v4 into shell iframes
…（TH ready / Zod wrap / YAML / SillyTavern / blob import 等）
d547dd7 feat(card-shell): ordered tavern_helper runtime
7239f33 feat(card-shell): extract test-card shells + host fetch + visible host
```

---

## 当前用户可见结果（2026-07-23）

**已通：**

1. TH 状态条：**6 全部完成**。
2. 状态栏壳：深色 Tab（任务/信息/持有物/命定/新闻/地图）可出，地图等子 UI 有按钮。
3. 不再卡在「iframe not ready / Vue/z/$/YAML/SillyTavern 未定义」连环死。

**未达金标 / 不对：**

1. **开场壳视觉错误**  
   - 用户金标期望：home 的「命定之诗与黄昏之歌 / Destined Poetry / 制作团队…」全页。  
   - 当前界面更像 **status 信息栏**（基础信息 / 种族无…）或 status 风格浅色 Tab，**不是**制作团队页。  
   - 可能原因（待查，**未修**）：  
     - `opening_home_url` 与 `status_bar_url` 挂载/缓存串味；  
     - home 依赖 `window.top.TavernHelper` / 世界书 / 扩展设置，沙箱 iframe 下走了失败分支只剩残缺 UI；  
     - 活动已有消息时开场挂载条件、`ShellAwareContent` 与写作 `#shell` 双挂导致错 URL。  
2. **变量真相**  
   - 壳内变量多在 ST selector / local 袋；与 Campaign/MVU 本局变量是否同源、开局 JSONPatch 是否灌进壳，**未验收**。  
3. **plugin-bridge 与壳宿主**  
   - 已注入 script，但壳仍用独立 blob iframe + 自有 `__sf_shell_bridge` 拉网；与 `PluginHost` 的事件/消息镜像是否完全同权，**未验收**。  
4. **GUI 证据**  
   - `scripts/run-card-shell-evidence.ps1` 偏单元/日志；金标截图闭环未完成。

**过程教训（写入计划，避免再犯）：**

- 仓库**已有** `plugin-bridge.js` ST 面时，禁止在 `CardShellHost` 无限堆手搓 free function「造轮子」。  
- 用户确认某一提交视觉正确时，**不要**未经确认回滚到旧 shim。  
- 重启/热更可能造成「同一代码看起来又坏」；应固定提交哈希 + 干净 dev 启动再判。

---

## 后续工作（仅文档清单，本轮不改代码）

1. **开场壳金标对齐**  
   - 运行时打印/UI 暴露实际 `opening_home_url` / `status_bar_url`；确认 home HTML 是否含「制作团队」。  
   - 排查 home 对 `window.top.*` 的依赖，改为同 iframe 的 `TavernHelper` 或宿主桥。  
   - 确认是否应优先 `custom_start`（创角）而非 `home`（制作组页）。  
2. **变量灌入**  
   - 开局 greeting 的 UpdateVariable/JSONPatch → 壳 `getVariables({type:'character'})` 可读。  
3. **证据**  
   - 金标：开场截图（制作团队页）+ 状态栏截图 + TH 6/6 截图归档 `docs/workstreams/TEST-CARD-SHELL-EVIDENCE.md`。  
4. **稳定**  
   - 去掉调试向「手搓 shim」分叉；单一 ST 面来源 = `plugin-bridge`。

---

## 非目标（仍成立）

- ST 99 事件全集、插件市场  
- knowledge 冒充世界书  
- 宣称与 ST 插件生态 100% 等价  

---

## 一句话

世界书分层与 TH 6/6、状态栏可渲染已到「能跑」；**test-card 开场金标页（制作团队）尚未对齐**，暂停功能改动，先以本文为进度真相源。

---

## 2026-07-23 续验更新（取代上述 home 错页判断）

- 已在真实 Tauri UI 复现并修复多 shell 同时存在时的 module source 抢答：status/home 不再互相加载对方源码。
- home 金标标题与制作团队内容已在首页壳和消息内首页壳显示；status 壳 Tab 与 TavernHelper 6/6 仍通过。
- “继续写作”已改为打开当前 Campaign 的最近会话，真实 UI 已恢复已采纳的 1,436 字会话。
- home 环境检查现在能终止并显示实际诊断，而不是永远“加载中”。未提供的 TavernHelper 版本、EJS 与 MVU 能力明确报告为未找到/未检测到/异常，不能记为完整 ST 兼容。

后续的唯一兼容性缺口是把上述三项接到真实 StoryForge 运行态；在此之前不应把环境检查写成通过，也不应把当前 card shell 宣称为 100% SillyTavern 等价。

---

## 2026-07-23 完成记录（取代上方兼容性缺口）

`test-card.png` 所需的三项能力现已接入真实运行态：

1. **TavernHelper + 活动世界书**：`getCharWorldbookNames('current')` 返回
   `storyforge:campaign:<id>`，`getWorldbook` 读取 Campaign 世界书的真实条目名与
   enabled 状态，`updateWorldbookWith` 只写回发生变化的 `disabled`。Rust DTO 保留
   ST 的 `extra.comment` 作为条目名；写回命令不重写正文、关键词或 route。
2. **EJS**：壳在宿主 allowlist 代持的 EJS 3.1.10 加载成功后，才把真实
   `render` 函数和已启用的 `extension_settings.EjsTemplate` 暴露给
   `SillyTavern.getContext()`。
3. **MVU**：`MvuJsRuntime` 只在其 iframe 真正报告 `mvu:ready` 后发布运行态；
   壳的 `waitGlobalInitialized('Mvu')` 通过桥接轮询该就绪态，超时仍会诚实失败。
4. **消息内壳上下文**：`ShellAwareContent` 也传递活动 `campaign-id`。这消除了
   顶层壳正常、消息内壳世界书为空的分裂状态。

真实 Tauri 复验确认首页环境检查会自动进入 DLC；DLC 和核心选择加载真实项目，
并完成“埃尔薇拉”禁用→保存→落盘→恢复的往返。状态栏、6/6 TH、多 Agent `cpa`
日志和已采纳会话亦在同一构建中可见。详见
`docs/workstreams/TEST-CARD-SHELL-EVIDENCE.md`。

仍不作超出范围的承诺：这不是“所有 ST 插件 API 均等价”的声明，而是本卡当前
依赖面已经有真实实现与桌面端证据。

### 并发完整性复查

- 当状态栏、开场和消息内壳同时挂载时，每个 iframe 使用独立 bridge session；非所属
  宿主不会响应其 `campaign_worldbook_update`。
- 同一 Campaign 的 shell 读取、差异计算和写回均在前端队列内执行，`CampaignStore`
  的世界书读改写则在持锁的原子 mutation 内落盘，避免多壳、过期快照或相邻 DLC
  开关覆盖彼此。
- `Disabled` 路由不能再被 UI 误报为已启用：可由 ST 蓝/绿灯恢复时恢复（双灯恢复为
  `Both`），否则返回可见错误。此项已有多宿主、过期快照、并发持久化和路由恢复回归测试。

---

## 2026-07-23 布局重排后的验收状态（最新）

### 已实现的布局合同

1. **完整开场在故事之前**：完整开场壳应出现在故事正文之前。
2. **状态跟随最后一条消息**：状态栏应紧随当前最后一条消息，而非固定在旧位置。
3. **TH 紧凑侧栏**：TavernHelper 使用紧凑侧栏布局。

### 已确认的质量证据

- `npm run test:all`：**343 Node tests + 44 Vitest tests** 通过。
- `npm run build`：通过。
- `scripts/run-card-shell-evidence.ps1`：通过。
- `cargo build -p storyforge`：通过。
- 第二轮代码复审：通过。

### 仍待完成：可交互桌面会话中的视觉位置复核

新版本 Tauri 已成功启动，并载入验收 Campaign，说明真实桌面构建可启动并可打开目标
数据。不过，Computer Use 尝试进入写作页时因 **`failed to activate captured window`**
无法激活已捕获窗口，未能执行点击操作。

因此，不能把完整开场、最后消息后的状态栏、TH 紧凑侧栏写成“已完成的真实交互视觉
位置验收”。它们的最终位置仍是一个明确的待办：在可交互桌面会话中进入写作页后复核。
本限制仅针对视觉/交互验收，不否定本节列出的构建、测试、脚本证据和代码复审结果。

---

## 2026-07-23 截图反馈后的布局修正（最新）

截图反馈后的实现已调整为以下布局行为：

1. **序章仅用于开局**：仅在尚未建立会话的开局阶段出现；会话建立后完全移除，不留下空白。
2. **受 session 保护的高度回传**：序章向宿主回传高度时校验 session，使容器随实际内容自然
   增高，并避免旧会话或其他宿主的回传影响当前布局。
3. **状态栏默认折叠且按需挂载**：初始为折叠态；点击展开才挂载、运行状态栏，关闭即卸载。

本轮质量证据：

- `npm run test:all`：**346 Node tests + 46 Vitest tests** 通过。
- `npm run build`：通过。
- 代码复审：通过。

---

## 2026-07-23 最终交互设计（最新）

1. **序章的显式授权条件**：仅“新建 Campaign”“选择角色”以及“继续写作但尚未有会话”三种
   流程可显示序章；历史会话不显示序章。
2. **状态栏的按需全屏入口**：状态栏改为右下悬浮球。点击后使用项目的 Headless UI `Dialog`
   全屏展开；关闭时卸载状态栏 iframe。
3. **Dialog 行为**：Dialog 使用 portal 与背景遮罩，支持 Esc/backdrop 关闭及焦点管理。

质量记录保持不变：`npm run test:all` **346 Node tests + 46 Vitest tests** 通过；
`npm run build` 通过；最终代码审查通过。

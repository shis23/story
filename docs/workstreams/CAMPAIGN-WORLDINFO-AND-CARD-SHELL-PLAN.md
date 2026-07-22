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

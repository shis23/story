# V5 应用级 CSP 收口 — RESULT（2026-07-27，返修版）

> 工作流：V5 应用级 CSP。分支 `codex/security-app-csp`。**未 push**。
> 结论：**PARTIAL** —— 静态 CSP、**真实 Chromium CSP 行为测试**、卡壳/MVU/PluginHost 兼容回归、前端构建与 Rust 编译全绿；WebView2 真机网络拦截实测受本代理环境限制未驱动（见 §7）。

## 0. SHA 与纪律

- base（开工 `main`）：`29513a600a404563ef5aad30ad52fa97b3d0c90a`
- 返修前 head：`65a0f1a852eace2d31bed3312179cba6b8956a06`（首轮：feature `656b9e7` + 文档 `65a0f1a`）。
- 返修后 head：见 §10 提交清单（`ac5b1e7` Rust → `b8bdac4` 前端+CSP → `9c5d919` 行为测试 → 本文档提交）。
- worktree：`.worktrees/security-app-csp`（`.worktrees/` 已 gitignore）。
- **本轮新增对 `crates/tauri-app/src/lib.rs` 的最小接线**（提示明确授权；详见 §3.2）。未还原 Android picker/数据目录、CI、SQLite、写作流水线改动。
- 未做：V6 权限统一。

## 1. 首轮设计错误与返修根因

首轮（`656b9e7`）把主应用 `csp: null` 换成收紧策略，但**未处理 iframe 文档的 CSP 继承**，导致阻断级回归：

- `default-src 'self'` + 无 `script-src` ⇒ 回落 `'self'`，禁内联脚本。
- 但 4 个运行时在 iframe 文档里都用**内联脚本**：
  - `CardShellHost.vue` / `TavernHelperRuntime.vue`：blob iframe + 内联 bridge。
  - `MvuJsRuntime.vue`：`srcdoc` + 内联 `SHIM_SCRIPT`。
  - `PluginHost.vue`：`srcdoc` + 内联 `bridgeScript`。
- **CSP Level 3 规定**（`w3c.github.io/webappsec-csp/#security-inherit-csp`）：`blob:` / `srcdoc:` / `data:` / `about:blank` 文档**继承创建者 policy container**；多重策略只能**交集**收紧，iframe 自身 `<meta>` **不能放宽**继承的策略。
- 因此首轮策略会**静默阻断所有 iframe 内联 bridge**，而 `buildShellCspMetaTag` 救不回。
- 首轮 RESULT 与 `appCsp.js` 注释关于 "shell never bounded by outer policy" 的表述**错误**，已删除修正。

### 1.1 可复现验证（真实 Chromium，非 jsdom/字符串检查）

`frontend/tests/csp-inheritance.spec.mjs`（Playwright 驱动真实 Chromium CSP 引擎，8 例全绿）证明：
1. blob iframe 内联脚本被继承主 CSP 阻断（复现回归）。
2. srcdoc iframe 内联脚本被阻断（MvuJsRuntime/PluginHost 形态）。
3. iframe 自身 `<meta>` CSP 无法救回被继承策略阻断的内联脚本（多重策略交集）。
4. 隔离 origin 文档独立 policy container 可正常运行内联脚本（修复形态）。
5. CardShell 风格 bridge 在隔离 origin 上 ready + ping/pong round-trip。
6. 主应用 origin `fetch('https://example.com')` 被主 CSP `connect-src` 阻断（V5 外联保证）。
7. 隔离 shell 可加载 `storyforge-cache` 协议资源，无 CSP 违规。
8. shell 自身 CSP 仍阻断未授权远程外联（即便内联脚本可运行）。

## 2. 修复：独立 origin 隔离文档

新增受限 Tauri 自定义协议 `storyforge-shell`，按一次性 token 提供文档 HTML，文档自带 shell CSP（HTTP 头 + `<meta>` 双重）。主应用 `frame-src` **只**允许该 origin。保留 `sandbox="allow-scripts"`（**不**加 `allow-same-origin`）。

### 2.1 三层 CSP 关系（正确模型）

| 层 | 谁设置 | 作用于 | 能否放宽上一层 |
| --- | --- | --- | --- |
| 父 CSP（主应用 `app.security.csp`） | Tauri 配置 | 主应用 origin 文档 | — |
| 继承 CSP | 自动 | 父创建的 blob/srcdoc/data/about:blank 子文档 | **不能**（只收紧） |
| iframe 自身 CSP（`<meta>` / 协议响应头） | shell 文档 / 协议 | 该文档 | **不能**（与继承策略交集，只收紧） |

**结论**：blob/srcdoc 形态下，iframe 内联脚本能否运行**取决于父 CSP**，shell 自身无法挽救。故必须让 shell 文档落在**不继承父 CSP 的独立 origin**（`storyforge-shell`），其 policy container 由协议响应头设定。

## 3. Rust 侧实现

### 3.1 新模块 `crates/tauri-app/src/shell_doc_protocol.rs`
- `pub const SHELL_DOC_SCHEME = "storyforge-shell"`；平台 origin 常量（镜像 `card_shell_cache.rs:26-29`）：Win/Android `http://storyforge-shell.localhost`，macOS/Linux `storyforge-shell://localhost`。
- 进程内 token registry：`static REGISTRY: OnceLock<Mutex<HashMap<String, Arc<String>>>>`（镜像 `get_card_shell_cache()` lib.rs:159 的 OnceLock 单例）。`register_shell_doc(html) -> token`（32 字节随机 hex）。**不持久化**：shell 文档是父进程内存中内容的临时渲染面，重启无物可恢复。
- `shell_doc_protocol_response(request)`：镜像 `card_shell_cache_protocol_response`（lib.rs:11763-11804）——只接受 GET/HEAD/OPTIONS；`path().trim_start_matches('/')` 作 token；严格校验（64 hex，拒遍历/空/斜杠）；命中返回 `200 text/html; charset=utf-8` + CORS 头 + **`Content-Security-Policy` 头**（= shell 策略，与 `buildShellCspContent([])` 同义）+ `Cache-Control: no-store`；未命中/非法 → 404（不带 CSP 头）。
- 单测 8 例（全绿）：注册取回含 CSP 头、HEAD 空体、未知 token 404 不带 CSP、非法/遍历 token 404、非 GET 405、OPTIONS 预检 204、同 HTML 两次注册得不同 token、token 校验器严格性。

### 3.2 `lib.rs` 最小接线（提示授权；与 Android 线的冲突协议）
- `mod shell_doc_protocol;`（lib.rs 模块声明区）。
- `tauri::Builder::default()` 链上新增一行 `.register_uri_scheme_protocol(shell_doc_protocol::SHELL_DOC_SCHEME, ...)`（紧随 `storyforge-cache` 注册之后）。
- 新 Tauri 命令 `card_shell_register_doc(html) -> Result<String, String>`（挂入 `invoke_handler`）。
- **未改 Android picker/数据目录/任何现有命令逻辑**。
- **与 Android 线的 rebase/cherry-pick 协议**：`lib.rs:13328` 附近的 Builder 链是 Android 线共享热点。本线改动是**纯增量**（一行协议注册 + 一个独立命令 + 一个模块声明）。若 Android 线同区域改动，合入时按 Android-first 顺序 cherry-pick 本线的协议注册行与命令声明；模块声明独立无冲突。

## 4. 前端迁移

### 4.1 `frontend/src/utils/shellDocUrl.js`（新）
封装 `registerShellDoc(html)` → `invoke('card_shell_register_doc')` → 返回 `<SHELL_DOC_ORIGIN>/<token>`；导出平台 origin（与 Rust 同步）+ `configureShellDocInvoke(invoke)`（注入依赖，保持可单测）+ `shellDocUrlForToken(token)`（测试 fixture）。

### 4.2 4 个运行时迁移（均保留 `sandbox="allow-scripts"`，不加 allow-same-origin）
| 组件 | 原 | 迁移后 |
| --- | --- | --- |
| `CardShellHost.vue` | blob URL（`URL.createObjectURL`） | `registerShellDoc(wrappedHtml)`；`:src`；保留 shell CSP meta（原有） |
| `TavernHelperRuntime.vue` | blob URL | 同上；**新增** shell CSP meta（原无） |
| `MvuJsRuntime.vue` | `:srcdoc` 常量内联脚本 | `buildMvuShellDoc()` + `registerShellDoc`；`:src`（onMounted 解析）；**新增** shell CSP meta |
| `PluginHost.vue` | `:srcdoc` computed | `iframeDoc` computed + `watch` 重注册；`:src`；**新增** shell CSP meta |

非 Tauri 环境（happy-dom/Vitest/Playwright IPC mock 未覆盖该命令时）回退 blob URL，保证测试与旧 harness 仍可渲染。

## 5. 最终 CSP 指令

`crates/tauri-app/tauri.conf.json` `app.security.csp`（与 `frontend/src/utils/appCsp.js` `APP_CSP` 字节一致，由 `app-csp.test.mjs` drift-guard 守门）：

```
default-src 'self';
connect-src ipc: http://ipc.localhost data: blob: http://storyforge-cache.localhost storyforge-cache://localhost;
img-src 'self' data: blob: http://storyforge-cache.localhost storyforge-cache://localhost;
media-src 'self' data: blob: http://storyforge-cache.localhost storyforge-cache://localhost;
font-src 'self' data: blob: http://storyforge-cache.localhost storyforge-cache://localhost;
style-src 'self' 'unsafe-inline';
frame-src http://storyforge-shell.localhost storyforge-shell://localhost data:;
object-src 'none';
form-action 'none';
base-uri 'none';
```

### 5.1 与首轮的差异（返修点）
- `frame-src`：`blob: data:` → **`http://storyforge-shell.localhost storyforge-shell://localhost data:`**（只允许隔离 origin；去掉 blob，因父创建的 blob 文档会继承本 CSP 阻断内联 bridge）。
- 删除 `worker-src 'self' blob:`：**无消费者**（无 worker 依赖）。回落 `default-src 'self'`。
- `script-src` 仍不写（回落 `'self'`），Tauri 编译期 hash 覆盖 `index.html` 内联主题脚本。

### 5.2 逐源消费者审计（每个非 self 源对应真实消费者）
| 源 | 消费者 |
| --- | --- |
| `ipc:` `http://ipc.localhost` | Tauri IPC（`frontend/src/tauri-api.js` 全部 `invoke`） |
| `http://storyforge-cache.localhost` `storyforge-cache://localhost` | `card_shell_cache.rs` 大资源本地协议（两平台形态） |
| `http://storyforge-shell.localhost` `storyforge-shell://localhost` | `shell_doc_protocol.rs` 隔离 shell 文档（4 个运行时 iframe） |
| `data:` `blob:` | 内存资源（各指令） |
| `'unsafe-inline'`（仅 style-src） | Vue scoped CSS + 主题 bootstrap |

### 5.3 DRIFT GUARD 说明（修正表述）
`APP_CSP` 与 `tauri.conf.json` 是**手动维护的字节级副本**，**非构建时生成**，故**不是真正单一真相源**——它是 drift guard，由 `app-csp.test.mjs` 断言相等，漂移即构建失败。`appCsp.js` 注释已据此修正。

## 6. 被阻断场景（CSP 生效后）
主应用 origin 下：`fetch`/`XHR`/`WebSocket`/`sendBeacon` 直连外网、`<img src=https://...>` 外泄、`<script src=https://...>`、`<iframe>/<object>/<embed>` 远程文档（`object-src 'none'` + `frame-src` 仅隔离 origin）、`<form>` 外网提交（`form-action 'none'`）、`<base>` 劫持（`base-uri 'none'`）。隔离 shell 内未授权外联由 shell 自身 CSP 阻断（见 §1.1 测试 8）。

## 7. 测试与验证（必跑项结果）

| 命令 | 结果 |
| --- | --- |
| `cd frontend && npm test`（Node `--test`，含 `app-csp.test.mjs` 12 例） | **443 pass / 0 fail** |
| `cd frontend && npm run test:ui`（Vitest，CardShell/RichContent/ShellAwareContent/MVU/PluginHost 组件） | **74 pass / 0 fail**（16 文件） |
| `npx playwright test --config=playwright.csp.config.mjs`（**真实 Chromium CSP 行为**，8 例） | **8 pass / 0 fail** |
| `cd frontend && npm run build` | **成功**（`built in 4.28s`），`dist/` 无 `.wasm`、无远程 script src |
| `cargo check -p storyforge` | **Finished**（tauri-build 接受新 CSP + 新协议） |
| `cargo test -p storyforge --lib shell_doc_protocol` | **8 pass / 0 fail** |
| `cargo build -p storyforge --bin storyforge` | **成功**（1m57s，`target/debug/storyforge.exe`）→ 二进制嵌入新协议 + 命令 + 生效 CSP |
| `git diff --check` | **CLEAN** |

## 8. WebView2 smoke 层级 — PARTIAL（诚实）
本代理环境**未真机驱动 WebView2**。所做"接近证据"：`cargo check`/`cargo build` 通过 → 新协议与命令编译进产物；真实 Chromium CSP 行为测试（§1.1）证明规范级行为。**未做**：可见 WebView2 窗口实测（主应用渲染无 CSP 违规、`fetch('https://...')` 被拦、真实卡 CardShell 执行、MVU/PluginHost ready）。按提示要求，**不**把 Chromium/Node 测试写成 WebView2 PASS。待主会话/用户实测项：见 §9。

## 9. Android / WebView2 待验项
- **WebView2 真机**：启动产物，确认 (a) 主应用渲染无 CSP 违规；(b) DevTools Console 无 `Refused to ...`；(c) `fetch('https://example.com')` 被主 CSP 拦并报 `connect-src` 违规；(d) 加载真实卡（Destiny/卿卿）CardShell 正常执行；(e) MVU 与 PluginHost ready。
- **Android WebView**：`http://storyforge-shell.localhost` 形态生效、卡壳渲染、MVU/PluginHost ready、网络拦截（与 Android 线并行，待其 RESULT 汇总）。
- **blob iframe CSP 继承在 WebView2/Android 的实际行为**：架构评审 V5 建议；本线已用隔离 origin 规避该不确定性（shell 文档不再依赖继承），但真机回归仍需确认 `storyforge-shell` 协议在两端正常注册与响应。

## 10. 提交清单（独立逻辑提交，不 push）
返修分提交：
1. **Rust**（`ac5b1e7`）：`shell_doc_protocol.rs` + `lib.rs` 最小接线（协议注册 + 命令 + 模块声明）+ 单测。
2. **前端 + CSP**（`b8bdac4`）：`shellDocUrl.js` + 4 运行时迁移 + `appCsp.js`/`tauri.conf.json`/`app-csp.test.mjs` 修正。
3. **行为测试**（`9c5d919`）：`csp-inheritance.spec.mjs` + `playwright.csp.config.mjs`。
4. **文档**（本提交）：本 RESULT（修正安全模型、SHA、消费者审计、Android 协议）。

**分类**：feature commit = `ac5b1e7`、`b8bdac4`（code-under-test）；test commit = `9c5d919`；doc commit = 本提交。

> `crates/tauri-app/gen/schemas/*.json` 仅 CRLF 空白变化（`git diff --ignore-all-space` 为空），**不**入提交。

## 11. 剩余风险
1. **WebView2/Android 真机未实测**（§8）：编译期与 Chromium 行为正确 ≠ WebView2 运行时已验。
2. **`style-src 'unsafe-inline'`** 仍为唯一放宽。
3. **lib.rs Builder 链**与 Android 线共享：纯增量，但合入需按 Android-first cherry-pick（§3.2）。
4. shell 文档 token registry **不持久化**：重启后旧 shell URL 失效（符合预期，shell 是临时渲染面）。
5. 本策略不解决 V6（插件越权）——另一条线。

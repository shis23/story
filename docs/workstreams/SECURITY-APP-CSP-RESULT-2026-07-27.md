# V5 应用级 CSP 收口 — RESULT（2026-07-27）

> 工作流：V5 应用级 CSP。分支 `codex/security-app-csp`。**未 push**。
> 结论：**PARTIAL** —— 静态 CSP、卡壳兼容回归、前端与 Rust 配置门全绿；WebView2 真机网络拦截实测受本代理环境限制未驱动（见 §6）。

## 0. SHA 与纪律

- base（开工 `main`）：`29513a600a404563ef5aad30ad52fa97b3d0c90a`
- head：`656b9e713c6b94ac5cfc31fa428403a37883474f`（`feat(security): pin main-app CSP and add contract tests (V5)`）。
- worktree：`.worktrees/security-app-csp`（`.worktrees/` 已 gitignore）。
- 未修改：`crates/tauri-app/src/lib.rs`、Android picker/manifest/capabilities、`.gitea/workflows/**`、`EXECUTION-PROGRAM-2026-07-27.md`。
- 未做：V6 权限统一、`tauri-app/src/lib.rs` 巨石拆分。

## 1. 资源通道清单（核实结论）

主应用（受信 UI origin）的实际资源通道，逐项代码核实：

| 通道 | 消费者 | CSP 处理 |
| --- | --- | --- |
| 应用 bundle（Vite 产物） | 整个 UI | `default-src 'self'`（dev：`http://localhost:1420`，prod：Tauri custom-protocol origin） |
| Tauri IPC（所有后端调用） | `frontend/src/tauri-api.js` 全部 `invoke(...)`：LLM、角色/预设导入、对话、密钥（keyring） | `connect-src ipc: http://ipc.localhost` |
| 自定义缓存协议 `storyforge-cache` | 大资源（卡地图等）经 `card_shell_cache.rs` 写盘后由本地协议下发 | `connect-src`/`img-src`/`media-src`/`font-src` 含 `http://storyforge-cache.localhost`（Win/Android）与 `storyforge-cache://localhost`（macOS/Linux）两形态 |
| `data:` / `blob:` | 内联资源、shell blob 文档、内存响应 | `connect-src`/`img-src`/`media-src`/`font-src` 含 `data:` `blob:` |
| blob iframe | `CardShellHost.vue:204` 父进程 `new Blob([html])` + `createObjectURL` 加载 `sandbox="allow-scripts"` 卡文档 | `frame-src blob: data:` |
| 远程图片/字体（主应用直连） | **无**。主应用不直连任何远程主机（LLM 走 Rust 后端） | 故不放开任何 `https://` |
| 远程 CDN 脚本（jQuery/Vue/zod/Ejs/lodash/js-yaml） | `CardShellHost.vue:643-696`、`TavernHelperRuntime.vue:303-451` | **不属主应用 CSP**：经 `__sfHostFetchText`/`__sfThHostFetchText` → `ask('fetch_text')` postMessage → Rust `card_shell_cache.rs`（含 SSRF + allowlist），在 **shell iframe 内** 以内联脚本注入；由 shell 自身 CSP（`cardShellCsp.js`，M3 已修）约束 |
| 卡壳 fetch proxy | `cardShellFetchProxy.js` 生成 shell 内 `window.fetch` monkey-patch，转 `__sfHostFetchDataUrl` | shell 内脚本，受 shell CSP 约束，与主应用 CSP 无关 |

**关键不变量**：主应用 origin **零** 直连网络出口。所有远程资源需求都经 host-mediated fetch（Rust 侧 allowlist + IP 字面量/localhost 拒绝 + 重定向逐跳校验）。

## 2. 最终 CSP 指令

`crates/tauri-app/tauri.conf.json` `app.security.csp`（与 `frontend/src/utils/appCsp.js` `APP_CSP` 字节一致，由 `app-csp.test.mjs` 守门）：

```
default-src 'self';
connect-src ipc: http://ipc.localhost data: blob: http://storyforge-cache.localhost storyforge-cache://localhost;
img-src 'self' data: blob: http://storyforge-cache.localhost storyforge-cache://localhost;
media-src 'self' data: blob: http://storyforge-cache.localhost storyforge-cache://localhost;
font-src 'self' data: blob: http://storyforge-cache.localhost storyforge-cache://localhost;
style-src 'self' 'unsafe-inline';
frame-src blob: data:;
worker-src 'self' blob:;
object-src 'none';
form-action 'none';
base-uri 'none';
```

Tauri 2 行为（官方 CSP 文档 + issue #3583）：`dangerousDisableAssetCspModification` 默认 `false`，故 `tauri-build`/`tauri-codegen` 在编译期自动为 `index.html` 的内联 `<script>`（本仓即主题 bootstrap 内联脚本）追加 SHA hash 到 `script-src`，并在生产注入 nonce。因此本策略 **不** 显式写 `script-src`（回落到 `default-src 'self'`），由 Tauri 在编译期补 hash/nonce。

### 2.1 逐项放宽审计（消费者 + 威胁边界）

| 源 | 指令 | 消费者 | 威胁边界 / 为何必须 |
| --- | --- | --- | --- |
| `'self'` | default-src | 应用 bundle | 基线，仅本地产物 |
| `ipc:` `http://ipc.localhost` | connect-src | Tauri IPC | Tauri 强制要求；IPC 是唯一后端通道 |
| `http://storyforge-cache.localhost` | connect/img/media/font-src | `card_shell_cache.rs` 大资源本地协议（Win/Android 形态） | 本地注册协议，文件名由 sha256 派生 + 反遍历（`card_shell_cache.rs:195-213,421-428`）；非外网 |
| `storyforge-cache://localhost` | 同上 | 同上（macOS/Linux 形态） | 同上 |
| `data:` `blob:` | 多指令 | 内存资源、shell blob 文档 | 非网络；blob 指向进程内对象 |
| `'unsafe-inline'`（仅 style-src） | style-src | Vue scoped CSS 注入 + `index.html` 主题 bootstrap | **仅样式**；脚本未授予 unsafe-inline（Tauri 编译期 hash 覆盖） |
| `frame-src blob: data:` | frame-src | `CardShellHost.vue` shell iframe | 卡壳渲染必需；shell 自带更严 CSP |
| `worker-src 'self' blob:` | worker-src | 潜在 bundled/blob worker | 预留；非网络 |

### 2.2 明确未授予（且有测试断言其不存在）

- 任何 `https://` / `http://` 远程主机（`http://ipc.localhost` 与 `http://storyforge-cache.localhost` 除外，二者均为本地）。
- `*`。
- `script-src 'unsafe-inline'` / `'unsafe-eval'`（无 WASM、无 eval 需求；`package.json` 无 wasm/onnx/sqlite 依赖，`dist/` 无 `.wasm`）。

## 3. 被阻断场景（CSP 生效后）

主应用 origin 下的以下外联将被 CSP 拦截（修复前 `csp:null` 全部畅通）：

- 卡外脚本通过 `XMLHttpRequest`/`fetch` 直连任意外网（V5 核心缺口）。
- `<img src="https://attacker/...">` / `new Image().src` 外泄（H3/L1 渲染正则注入路径，主应用层）。
- `navigator.sendBeacon`、`WebSocket` 到外网。
- `<script src="https://...">` 远程代码加载。
- `<iframe>`/`<object>`/`<embed>` 加载远程文档或插件（`object-src 'none'` + `frame-src` 仅 blob:/data:）。
- `<form>` 向外网提交（`form-action 'none'`）。
- `<base>` 劫持相对 URL（`base-uri 'none'`）。

> 注：卡壳 iframe 内的外联由 **shell CSP**（`cardShellCsp.js`，M3 已修）阻断；本策略是其外层补充，覆盖主应用 origin 直接发起的通道与 shell 文档的加载本身。

## 4. 兼容限制

1. **`style-src 'unsafe-inline'`**：Vue scoped CSS 与主题 bootstrap 必需。脚本未放宽。若将来要去掉，需把主题 bootstrap 改为外链或由 Tauri hash 注入覆盖（当前由编译期 hash 机制处理的是 `<script>`，非 `<style>`）。
2. **自定义缓存协议两形态并存**：策略同时列 `http://storyforge-cache.localhost` 与 `storyforge-cache://localhost`，确保同一 bundle 在 Win/Android 与 macOS/Linux 都可用（镜像 `card_shell_cache.rs:27-29` 与 `cardShellCsp.js` `SHELL_CACHE_ORIGINS`）。
3. **dev 模式**：`devUrl: http://localhost:1420` 属 `'self'` 在 Tauri dev 下的解析范畴；dev 下 Tauri 仍套用同一 CSP（未单独设 `devCsp`），与生产一致收紧。若 dev 工具链（如 HMR ws）触发拦截，后续可考虑加受限的 `devCsp`——本轮未引入，避免为便利放宽。

## 5. 测试

### 5.1 新增静态 CSP 契约测试（TDD fail-first → green）

`frontend/tests/app-csp.test.mjs`（10 例，全绿）。重点：

- **fail-first 守门**：`tauri.conf.json app.security.csp equals APP_CSP byte-for-byte` —— 在配置还是 `null` 时此例失败（已实测），配置更新后转绿。防"文档写了、配置仍 null"漂移。
- **V5 回归守卫**：`NO directive grants arbitrary remote egress` —— 断言无 `*`/`https:`/`http:`（除两个 localhost）/`unsafe-eval`，且 `unsafe-inline` 仅 style-src。
- **script-src 不含 unsafe-inline/unsafe-eval**。
- **object-src/form-action/base-uri 锁死 'none'**。
- **frame-src 允许 blob:/data:**（shell iframe 必需）。
- **缓存协议源镜像** `card_shell_cache.rs` + `cardShellCsp.js`。
- **shell 兼容回归**：外层策略不收窄 shell 依赖的 blob iframe 与缓存协议通道。

`frontend/src/utils/appCsp.js` 导出 `APP_CSP`（与配置同款）、`APP_CACHE_ORIGINS`、`parseCsp`，既是测试锚点也是可审计的策略单一真相源。

### 5.2 全量回归（命令 + 结果）

| 命令 | 结果 |
| --- | --- |
| `cd frontend && npm test`（Node `--test`，含新 `app-csp.test.mjs`） | **441 pass / 0 fail** |
| `cd frontend && npm run test:ui`（Vitest，含 CardShell/RichContent/ShellAwareContent/MVU/PluginHost） | **74 pass / 0 fail**（16 文件） |
| `cd frontend && npm run build`（生产构建） | **成功**（`built in 4.08s`），`dist/` 无 `.wasm`，无远程 `src=`，仅 1 个内联主题 `<script>`（由 Tauri 编译期 hash 注入覆盖） |
| `cd .worktrees/security-app-csp && cargo check -p storyforge` | **Finished**（4m08s）—— `tauri-build`/`tauri-codegen` 接受新 CSP 字符串 |
| `git diff --check` | **CLEAN** |
| `node --test tests/card-shell-csp.test.mjs`（shell CSP 跨策略回归） | **4 pass / 0 fail** |

关键 UI 回归点：`card-shell-host-sandbox.test.mjs`（"wraps inline html into a CSP-pinned bridge document inside a sandboxed iframe"）、`shell-aware-content.test.mjs`（11 例）、`card-shell-floating-status`、`writing-screen-runtime-slots` 全绿——卡壳挂载链路未被外层 CSP 破坏。

## 6. WebView2 smoke 层级 — PARTIAL

**诚实声明**：本代理环境**未能真机驱动 WebView2 窗口**进行运行时网络拦截实测。依据提示词第 37 行，不以浏览器/单元测试冒充 WebView2/Android 真机验证。

实际做到的"接近证据"：

- `cargo check -p storyforge` 通过 → `tauri-build`/`tauri-codegen` 在编译期成功解析新 CSP（无效 CSP 字符串会让 build.rs 失败）。
- `cargo build -p storyforge --bin storyforge` **成功**（6m39s，`target/debug/storyforge.exe` 49MB）→ debug 二进制嵌入了生效的 CSP（编译期 codegen 把 CSP 注入产物）。**但未**在可见 WebView2 窗口里实测拦截行为。
- `npm run build` 产出与 CSP 假设一致的 bundle（无 wasm、无远程 script src、仅 1 个会被 Tauri hash 覆盖的内联脚本）。

**未做到 / 待主会话或用户实测**：

- 启动 `tauri dev` 或安装产物，在真实 WebView2 中：(a) 主应用正常渲染与交互；(b) DevTools Console 无 CSP 违规；(c) 手工触发一个被禁外连（如 console 执行 `fetch('https://example.com')`）确认被拦截并报 CSP 违规；(d) 加载一张真实卡（如 Destiny/卿卿）确认 shell iframe + 缓存协议 + CDN host-fetch 链路未受外层 CSP 影响。
- Android WebView 真机：`http://storyforge-cache.localhost` 形态生效、卡壳渲染、网络拦截（本轮 Android 线并行，待 Android 线 RESULT 汇总）。

## 7. Android 待验项

- 本策略同时列 `http://storyforge-cache.localhost`，匹配 Android WebView 下 Tauri/Wry 的自定义协议映射（`card_shell_cache.rs:27` cfg）。
- Android WebView 对 blob iframe 的 CSP 继承行为需真机确认（架构评审 V5 建议点）。卡壳另有自身 `<meta>` CSP 作为继承不可靠时的兜底（M3 已修，`CardShellHost.vue` headInject）。
- 本轮不改 Android picker/manifest/capabilities；与 Android 线无文件冲突。

## 8. 安全与代码审查

- 策略单一真相源：`tauri.conf.json`（运行时）与 `appCsp.js`（测试锚点）字节一致，由测试守门。
- 无 `*`、无任意 `https:`、无 `unsafe-eval`、`script-src` 无 `unsafe-inline`。
- 每个非 `'self'` 源均有消费者与威胁边界记录（§2.1）。
- 未为测试方便关闭 CSP；未把单元/构建测试写成视觉/网络实测（§6 如实标 PARTIAL）。
- `git diff --check` clean；未 push。

## 9. 提交

独立逻辑提交（不 push）：
- `feat(security): pin main-app CSP and add contract tests` —— `tauri.conf.json` + `appCsp.js` + `app-csp.test.mjs` + 本 RESULT。

> 生成的 `crates/tauri-app/gen/schemas/*.json` 仅 CRLF 空白变化（`git diff --ignore-all-space` 为空），**不** 入本次提交。

## 10. 剩余风险

1. **WebView2 运行时未实测**（§6）：编译期解析正确 ≠ 运行时拦截行为已验。任何"已阻断"结论在真机复测前应视为高置信未验。
2. **`style-src 'unsafe-inline'`** 是当前唯一放宽；若未来引入更严 CSP 工具链需先重构主题 bootstrap。
3. **dev 模式 HMR**：若 dev 下 Vite HMR ws 触发 CSP 拦截，需加受限 `devCsp`（本轮未引入）。
4. 本策略不解决 V6（插件 `get_conversation` 越权 / `start_writing` 误匹配）——那是另一条线，本轮明确不扩。

---

**head SHA**：`656b9e713c6b94ac5cfc31fa428403a37883474f`（见 `git log -1 codex/security-app-csp`）。

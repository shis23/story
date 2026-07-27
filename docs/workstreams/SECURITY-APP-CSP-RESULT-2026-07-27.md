# 应用级 CSP 隔离 — 合并复核结果（2026-07-27）

## 结论

分支 `codex/security-app-csp`（head `49c6876`）已通过 merge commit `70a2429`
合入本地 `main`。隔离 origin 方案成立，合并复核又修复了资源生命周期、内存上限、
token 强度和浏览器/HMR 兼容问题。

编译、Node/Vitest、真实 Chromium CSP 行为和本地浏览器 smoke 均通过；WebView2 与
Android WebView 真机仍为 **BLOCKED**。

## 最终安全模型

- 主应用 `frame-src` 只允许
  `http://storyforge-shell.localhost` 与 `storyforge-shell://localhost`。
- 主应用不再允许 `data:` iframe；`blob:`/`srcdoc:`/`about:blank` 的 CSP 继承问题
  通过独立 origin 规避。
- shell 文档携带自身 CSP，继续以 `sandbox="allow-scripts"` 运行，不授予
  `allow-same-origin`。
- shell token 使用两个 `Uuid::new_v4()` 拼成 64 位小写十六进制随机值。
- registry 上限为 128 个文档，单文档最大 16 MiB。
- GET 原子取出并消费 token；重放返回 404。HEAD 只探测，不消费。
- 前端卸载、替换和异步竞态都会调用注销命令；blob fallback 也会 revoke。
- 协议响应不再发送不必要的 `Access-Control-Allow-Origin: *`，并设置
  `Cache-Control: no-store`。

## 合并后复核修正

- 增加显式 `card_shell_unregister_doc` 命令及前端 `releaseShellDoc`。
- 四个 iframe 消费者都处理“旧注册晚返回”的竞态，避免泄漏一次性文档。
- Tauri 生产环境注册失败时不再静默退回 blob；普通 Vite/test 环境才允许 fallback。
- `MvuJsRuntime` 只在 `window.__TAURI_INTERNALS__` 存在时订阅 Tauri event。此前普通
  浏览器/HMR 会在 mounted hook 抛出 `transformCallback` undefined；已用红→绿测试和
  浏览器重载日志验证。

## 验证

- `cargo test --workspace`：PASS。
- `cargo test -p storyforge --lib shell_doc_protocol`：10 passed。
- `npm test`：全绿（含 shell URL/token、MVU runtime bridge 回归）。
- `npm run test:ui`：25 files / 94 tests passed。
- `npm run build`：PASS。
- `npx playwright test --config=playwright.csp.config.mjs`：真实 Chromium 8/8 passed。
- 本地 `http://localhost:1420/` smoke：角色卡库、Campaign 管理空态可进入；修复后
  重载新增 warn/error 为 0。

## 尚未验证

- Tauri WebView2 中真实卡 CardShell、MVU/PluginHost ready 与网络拦截。
- Android WebView 中自定义协议注册、CSP 响应和运行时兼容。
- 因此这里不把 Chromium 或普通浏览器结果表述为 WebView2/Android 真机 PASS。

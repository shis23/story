# 应用级 CSP 隔离 — 最终验收结果（更新于 2026-07-28）

## 结论

应用级 CSP 隔离与 Windows WebView2 角色卡运行时已收口。主应用、隔离壳文档和
壳内 ES module 使用不同的受限资源通道；真实“命定之诗”角色卡的 CardShell
状态面板与 TavernHelper 脚本均已在 StoryForge 桌面程序中运行通过。

Android WebView 运行时仍未做模拟器或真机验证，因此 Android 证据保持
**BLOCKED**，不以 Windows 或 Chromium 结果代替。

## 最终安全模型

- 主应用 `frame-src` 只允许
  `http://storyforge-shell.localhost` 与 `storyforge-shell://localhost`。
- shell 文档由随机 64 位小写十六进制 token 定位，单次导航消费；HEAD 只探测。
- shell 文档继续以 `sandbox="allow-scripts"` 运行，不授予 `allow-same-origin`。
- 壳内模块由相同受限 origin 的 `/module/<token>` 路由提供：
  - JavaScript MIME；
  - 只允许 opaque sandbox 的 `Origin: null`；
  - `Cross-Origin-Resource-Policy: cross-origin`；
  - `Cache-Control: no-store`；
  - token 由宿主持有租约，壳替换或卸载时注销。
- registry 总上限为 128 个资源，单资源最大 16 MiB。
- 远程模块依赖由宿主代持抓取、解析和递归改写；壳 CSP 不开放公共
  `https:` 的 `script-src`/`connect-src`。
- 失败的模块图注册会释放已经创建的部分租约；动态 `import()` 的租约保留到
  iframe 生命周期结束，避免延迟导入在首次执行后失效。

## 本轮修复

- 修复旧正则漏识别压缩 ESM（例如 `from'…'`）而产生
  `Failed to fetch dynamically imported module` 的问题。
- 新增不执行卡片源码的词法扫描器，识别静态导入、再导出与字面量动态导入，
  同时忽略普通字符串和注释。
- TavernHelper 先区分经典脚本、import-only 包装和真实 ESM：
  - 经典脚本保持传统执行语义；
  - import-only 包装跟随到最终经典 bundle；
  - 真实 ESM 使用受限协议模块图。
- CardShell 与 TavernHelper 共用同一模块图注册器，不再各自维护脆弱的导入正则。
- 模块路由增加严格 token 校验、CORS、OPTIONS、重复 GET 与显式注销测试。

## 自动化验证

- `npm test`：468 passed。
- `npm run test:ui`：25 files / 94 passed。
- `npm run build`：PASS。
- `cargo fmt --all -- --check`：PASS。
- `cargo test -p storyforge shell_doc_protocol --lib`：13 passed。
- `cargo build -p storyforge`：PASS。
- `npx playwright test tests/csp-inheritance.spec.mjs --config=playwright.csp.config.mjs`：
  Chromium 9/9 passed，包括 opaque sandbox 在无 `allow-same-origin` 时导入
  tokenized protocol module，以及未授权远程请求被 CSP 阻断。

## Windows WebView2 实跑

测试对象：本机 StoryForge debug 桌面构建、现有 Campaign `test`、角色卡
“命定之诗”。

- PASS：进入既有写作会话，角色卡应用正常挂载。
- PASS：TavernHelper 显示“1 全部完成”，脚本明细为 `ok`，可见按钮已挂载。
- PASS：打开“当前状态”后，初始化完成并渲染“任务 / 信息 / 持有物 / 命定 /
  新闻 / 地图”以及属性、资源与状态效果。
- PASS：旧版可稳定复现的
  `Failed to fetch dynamically imported module:
  http://storyforge-shell.localhost/module/...` 不再出现。
- PASS：运行时仍为 `sandbox="allow-scripts"`，没有通过
  `allow-same-origin` 绕过隔离。

## 尚未验证

- PluginHost 没有可用的真实第三方插件样本，本轮只覆盖编译、单元与 Chromium
  行为测试，未将其表述为 WebView2 插件实跑 PASS。
- Android WebView 的自定义协议注册、CSP 响应、模块 CORS 与卡片运行时需在
  emulator/physical device 上另行验收。

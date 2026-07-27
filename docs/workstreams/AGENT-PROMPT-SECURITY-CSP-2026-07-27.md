# 可复制提示词：V5 应用级 CSP 收口

你负责 StoryForge 的 **V5 应用级 CSP 收口**。你不是唯一在代码库工作的代理；Android 代理本轮拥有 `tauri-app/src/lib.rs`，Release CI 和桌面验收也在并行进行。不要还原或覆盖他人的改动。

## 开工要求

1. 建立独立 worktree/分支 `codex/security-app-csp`，从开工时最新 `main` 记录 base SHA。
2. 完整阅读：
   - `AGENTS.md`
   - `docs/workstreams/PARALLEL-WORKSTREAM-HANDOFF-2026-07-27.md`
   - `docs/workstreams/ARCHITECTURE-REVIEW-2026-07-26.md` 的 V5
   - `docs/workstreams/CARD-SHELL-REVIEW-2026-07-26.md` 的 M3/H3 及修复状态
   - `frontend/src/utils/cardShellCsp.js`、卡壳构造、RichContent/ShellAwareContent 和 `crates/tauri-app/tauri.conf.json`
3. 先确认包装文档 CSP 已完成哪些能力，避免把壳内 CSP 与主应用 CSP 混为一谈。

## 目标

把主应用 `csp: null` 推进为可审计的应用级网络边界，同时保持当前受支持的本地资源、`data:`/`blob:`、Tauri IPC、自定义缓存协议和卡壳包装文档可用。重点收紧 `connect-src`、`img-src`、`media-src`、`font-src`、`frame-src`、`form-action` 和对象/基址能力；对 `script-src`/`style-src` 的兼容例外必须逐项解释并有测试。

## 所有权与禁止项

你拥有：

- `crates/tauri-app/tauri.conf.json`
- CSP 生成/验证工具与测试
- 前端资源 URL 安全策略和必要的卡壳兼容测试
- 本工作流 RESULT

禁止修改：

- `crates/tauri-app/src/lib.rs`
- Android picker/manifest/capabilities
- `.gitea/workflows/**`
- 插件权限词表、V6 后端命令门禁、写作流水线、SQLite
- `docs/workstreams/EXECUTION-PROGRAM-2026-07-27.md`

不得用 `*`、任意 `https:` 或无解释的宽域名恢复功能；不得为测试方便关闭 CSP；不得声称浏览器单测等价于 WebView2/Android 真机验证。

## 必做事项

1. 写一份当前资源通道清单：应用 bundle、Tauri IPC、自定义缓存协议、远程图片/字体、blob iframe、卡壳 fetch proxy。
2. 先增加静态 CSP 契约测试和至少一个“禁止未授权外连”的失败用例。
3. 配置最小应用级 CSP；若某类兼容必须放宽，记录具体消费者和威胁边界。
4. 跑 CardShell、RichContent、MVU/PluginHost 相关 Node/Vitest 测试和前端生产构建。
5. 尽可能做 WebView2 smoke；无法真实驱动时标记为 PARTIAL，不把单元测试写成视觉/网络实测。
6. 输出 `docs/workstreams/SECURITY-APP-CSP-RESULT-2026-07-27.md`，列出最终指令、允许源、被阻断场景、兼容限制和 Android 待验项。

## 验证与交付

运行相关前端测试、生产构建、Tauri capability/config 测试、必要的 Rust 编译门和 `git diff --check`。完成安全与代码审查，独立提交，不 push。

最终报告 base/head SHA、CSP 差异、实际 smoke 层级、测试、RESULT、commit 和剩余风险。不要顺手扩展到 V6 权限统一。

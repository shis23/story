# dsh-plugin-storyforge（PoC）

把 StoryForge 多 Agent 写作管线接进 DeepSeek Harness（DSH）的插件原型。目前处于 PoC
阶段，后端是**假管线**（固定脚本输出），用于验证 DSH 侧的隔离与交互；真实管线在 P2
通过 Rust sidecar 接入。

## 目录

| 文件 | 作用 |
|---|---|
| `lib/index.js` | 宿主半边：注册 provider `storyforge`（假管线适配器）、`/api/storyforge/*` 受保护路由 |
| `lib/agent.js` | StoryForge 预设里的 agent 作用域插件：每次模型请求钉到 `storyforge/pipeline` |
| `lib/client.js` | 浏览器半边：左侧栏「StoryForge」入口 + 调试面板 |
| `lib/shared.js` | 常量 |
| `storyforge.patch.yml` | 启动时叠加的补丁：激活宿主插件 + 新增 `storyforge` 预设（只有 persona，无工具） |

## 在隔离环境里运行

全部在独立的 `DSH_HOME` 下进行，不会碰 `~/.dsh` 和正在运行的 DSH。以下以
`C:/Users/Predator/dsh-poc` 为例，工作区目录 `C:/Users/Predator/dsh-poc/workspace`
（DSH 以启动时的当前目录作为默认工作区）。

1. 从 web 模板建 profile（不启动，只初始化并导出配置）：

   ```bash
   cd /c/Users/Predator/dsh-poc/workspace && DSH_HOME="C:/Users/Predator/dsh-poc" dsh --profile sfpoc --from-default-profile web --dump-config > /dev/null
   ```

2. 以 `link:` 依赖挂上插件：

   ```bash
   DSH_HOME="C:/Users/Predator/dsh-poc" dsh plugin --profile sfpoc add "link:C:/Users/Predator/ZCodeProject/storyforge/integrations/dsh-storyforge"
   ```

3. 启动（端口由系统分配，不自动开浏览器）：

   ```bash
   cd /c/Users/Predator/dsh-poc/workspace && DSH_HOME="C:/Users/Predator/dsh-poc" dsh --profile sfpoc --patch "C:/Users/Predator/ZCodeProject/storyforge/integrations/dsh-storyforge/storyforge.patch.yml" --port 0 --no-open
   ```

4. 打开输出里的 `dsh web: http://127.0.0.1:<端口>/?token=…` 链接；新会话时在预设下拉里选
   「StoryForge」。

## 开发循环

- 改 `lib/client.js`：刷新页面即可生效。
- 改 `lib/index.js` / `lib/agent.js` / 补丁：重启实例。
- 调试：左侧栏「StoryForge」面板列出最近的适配器调用（会话、用途、模型、消息数、工具），
  也可直接请求 `/api/storyforge/debug/calls`。

## PoC 要回答的问题

1. StoryForge 会话能否与普通 DSH 会话完全隔离（专属预设无工具、模型被钉住、视图与面板只在
   StoryForge 会话出现）。
2. StoryForge 管线能否完全通过 DSH 的模型运行，StoryForge 自身不持有任何 API key。
3. StoryForge 会话的创建、开场、分叉、归档、重启能否与 Campaign 保持一致。

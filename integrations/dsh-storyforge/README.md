# dsh-plugin-storyforge（PoC）

把 StoryForge 多 Agent 写作管线接进 DeepSeek Harness（DSH）的插件原型。目前处于 PoC
阶段，后端是**假管线**（固定脚本输出），用于验证 DSH 侧的隔离与交互；真实管线在 P2
通过 Rust sidecar 接入。

## 目录

| 文件 | 作用 |
|---|---|
| `lib/index.js` | 宿主半边：注册 provider `storyforge`（假管线适配器）、可选的调试回显 provider、`/api/storyforge/*` 受保护路由（含「新建故事」） |
| `lib/agent.js` | StoryForge 预设里的 agent 作用域插件：每次模型请求钉到 `storyforge/pipeline`；记录 agent 创建 / 分叉 |
| `lib/client.js` | 浏览器半边：左侧栏「StoryForge」首页（新建故事 + 调试信息）；StoryForge 会话里的视图标签、右栏面板、会话头按钮、停靠条、模型锁 |
| `lib/shared.js` | 常量与进程内共享状态（生命周期事件、分叉回调） |
| `storyforge.patch.yml` | 启动时叠加的补丁：激活宿主插件（`debugEcho: true` 时多挂调试回显 provider）+ 新增 `storyforge` 预设（只有 persona，无工具） |

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

4. 打开输出里的 `dsh web: http://127.0.0.1:<端口>/?token=…` 链接；在左侧栏「StoryForge」
   首页点「新建故事」（建在 `stories` 工作区、默认停在 StoryForge 视图并打开右栏面板），
   或在新会话页的预设下拉里选「StoryForge」。

临时实例没有任何凭据时，补丁里的 `debugEcho: true` 会多注册一个本地回显 provider
`sf-debug-echo`（模型选择器里的「调试回显」），用来模拟普通会话：它回显本轮用户输入并报告
收到的工具数，不发任何外部请求。正式使用时去掉。

## 开发循环

- 改 `lib/client.js`：刷新页面即可生效。
- 改 `lib/index.js` / `lib/agent.js` / 补丁：重启实例。
- 调试：左侧栏「StoryForge」首页列出最近的适配器调用（会话、用途、模型、消息数、工具、
  跳过的注入消息）与 StoryForge 会话的生命周期事件；`/api/storyforge/debug/calls` 另含每次
  调用末尾几条消息的来源结构。

## 已知限制

- StoryForge 会话里的模型选择器被换成只读的「StoryForge 管线」。`/model` 命令仍能改选择：
  请求照样被钉回管线，但 DSH 会每轮在会话历史里追加一条换模型通知，并改掉全局默认模型。
- 补丁叠加的插件不出现在插件管理页，不能热停用。去掉补丁重启后，StoryForge 会话变为只读
  （发送报 `Unknown agent preset: storyforge`），历史不受影响；带回补丁即恢复。

## PoC 要回答的问题

1. StoryForge 会话能否与普通 DSH 会话完全隔离（专属预设无工具、模型被钉住、视图与面板只在
   StoryForge 会话出现）。
2. StoryForge 管线能否完全通过 DSH 的模型运行，StoryForge 自身不持有任何 API key。
3. StoryForge 会话的创建、开场、分叉、归档、重启能否与 Campaign 保持一致。

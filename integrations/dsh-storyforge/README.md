# dsh-plugin-storyforge（PoC）

把 StoryForge 多 Agent 写作管线接进 DeepSeek Harness（DSH）的插件原型。

StoryForge 会话的每一轮交给 Rust sidecar `storyforge-dsh-bridge`（`crates/dsh-bridge`）。
sidecar 跑的是与 Tauri 命令同一份生产代码：导演 / 角色 / 执笔、质量门与 autofix、后处理，
后处理结束后自动接受。管线内部的每一次模型调用都反向发回插件，由插件用你在 StoryForge
首页选的底层模型经 `ctx.llm.stream` 执行。所以 StoryForge 自身不持有任何 API key。
插件配置 `fakePipeline: true` 时退回假管线（固定脚本，不需要 Rust 和凭据）。

## 目录

| 文件 | 作用 |
|---|---|
| `lib/index.js` | 宿主半边：注册 provider `storyforge`（真实管线适配器，或假管线）、可选的调试回显 provider、`/api/storyforge/*` 受保护路由（新建故事、角色卡列表、底层模型选择、sidecar 状态、调试） |
| `lib/bridge.js` | sidecar 管理：懒启动、按行收发 JSON-RPC、执行 sidecar 发来的 `llm.stream`、转发管线事件 |
| `lib/agent.js` | StoryForge 预设里的 agent 作用域插件：每次模型请求钉到 `storyforge/pipeline`；记录 agent 创建 / 分叉 |
| `lib/client.js` | 浏览器半边：左侧栏「StoryForge」首页（新建故事、管线设置、我的故事、最近的收尾、底层模型调用、折叠的调试信息）；StoryForge 会话里的视图标签、右栏面板、会话头按钮、停靠条、模型锁。颜色与圆角取 DSH 主题变量，跟随亮 / 暗主题 |
| `lib/roles.js` | 按系统提示词里的身份句给每次底层调用标管线角色（导演、角色 · 名字、执笔、后处理……） |
| `lib/shared.js` | 常量与进程内共享状态（生命周期事件、分叉回调） |
| `storyforge.patch.yml` | 启动时叠加的补丁：激活宿主插件（`debugEcho: true` 时多挂调试回显 provider）+ 新增 `storyforge` 预设（只有 persona，无工具） |
| `test/host-smoke.test.mjs` | 宿主半边冒烟：假 DSH ctx + 假底层模型驱动真实 sidecar 跑一轮 |
| `test/roles.test.mjs` | 管线角色识别 |

插件配置（补丁里 `config` 下，均可省略）：

| 键 | 默认 | 说明 |
|---|---|---|
| `dataDir` | `$DSH_HOME/storyforge` | 会话映射（含已写轮数与最近一次收尾）、设置、收尾报告（`settles.json`）、日志 |
| `cardsDir` | `<dataDir>/cards` | 「新建故事」可选的角色卡（`.png` / `.json`） |
| `bridgeBin` | 仓库 `target/debug/storyforge-dsh-bridge(.exe)` | sidecar 可执行文件 |
| `bridgeDataDir` | `<dataDir>/bridge-data` | StoryForge 存储（JSON 后端），与桌面版数据完全分开 |
| `fakePipeline` | `false` | 用假管线代替 sidecar |
| `debugEcho` | `false` | 额外注册调试回显 provider |

sidecar 的 stderr 写到 `<dataDir>/logs/bridge.log`。StoryForge 自己的 LLM 调用日志在
`<bridgeDataDir>/logs/`。

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

3. 构建 sidecar，并把角色卡放进 `$DSH_HOME/storyforge/cards/`：

   ```bash
   cargo build -p storyforge-dsh-bridge
   ```

4. 启动（端口由系统分配，不自动开浏览器）：

   ```bash
   cd /c/Users/Predator/dsh-poc/workspace && DSH_HOME="C:/Users/Predator/dsh-poc" dsh --profile sfpoc --patch "C:/Users/Predator/ZCodeProject/storyforge/integrations/dsh-storyforge/storyforge.patch.yml" --port 0 --no-open
   ```

5. 打开输出里的 `dsh web: http://127.0.0.1:<端口>/?token=…` 链接。先在 DSH 设置里配好
   至少一个模型服务商（凭据由你在界面里填，插件不读任何凭据文件），再到左侧栏
   「StoryForge」首页：
   - 选「底层模型」：导演、角色、执笔、后处理都用它；
   - 「生成模式」默认自动路由；也可以显式选续写 / 双人戏 / 群像（并行或顺序子 Agent）。
     「工具协议」默认原生工具调用，模型不支持 function calling 时选「文本降级」（与
     StoryForge 连接的 TextFallback 同一套实现）；
   - 选角色卡，点「新建故事」：sidecar 导入卡、跑角色识别（会调用模型）、开档，然后打开
     新会话（建在 `stories` 工作区，默认停在 StoryForge 视图并打开右栏面板），第一条回复
     是卡的开场白；
   - 在会话里输入行动：思考块显示导演 / 角色 / 执笔的过程，末尾一行是本轮的模式与底层
     调用用量；回复正文是质量门之后的成文。会话里的用量是本轮正文阶段所有底层调用之和；
     后处理在后台继续，结束后自动接受（停靠条与面板显示结果和后处理用量）；
   - 首页「我的故事」按最近活动列出关联存档的故事（已写轮数、角色卡、分叉来源、最近一次
     接受结果），点整行打开；未关联存档的会话默认折叠。「最近的收尾」与故事进度落盘，DSH
     重启后仍在。「底层模型调用」表逐次列出管线发出的调用，「角色」列标出是导演、哪个角色、
     执笔还是后处理；
   - 从某条回复「分叉」：子会话第一次发送时映射成存档的分支（StoryForge 的 Campaign
     分支），之后两边各写各的。只支持从**最后一条**回复分叉。

   从新会话页的预设下拉里直接选「StoryForge」建的会话没有关联存档，发送会报错。

临时实例没有任何凭据时，补丁里的 `debugEcho: true` 会多注册一个本地回显 provider
`sf-debug-echo`（模型选择器里的「调试回显」），用来模拟普通会话：它回显本轮用户输入并报告
收到的工具数，不发任何外部请求。正式使用时去掉。

## 开发循环

- 改 `lib/client.js`：刷新页面即可生效。
- 改 `lib/index.js` / `lib/bridge.js` / `lib/agent.js` / 补丁：重启实例。
- 改 Rust：`cargo build -p storyforge-dsh-bridge` 后重启实例（sidecar 随插件拉起）。
- 测试：`cargo test -p storyforge-dsh-bridge`（含拉起 sidecar 进程、Mock 模型跑两轮的端到端
  测试）；`node --test integrations/dsh-storyforge/test/host-smoke.test.mjs integrations/dsh-storyforge/test/roles.test.mjs`
  （冒烟需先构建 sidecar）。
- 调试：左侧栏「StoryForge」首页底部的「调试信息」（默认折叠）列出写作引擎进程状态、最近的适配器调用（会话、用途、模型、消息数、工具、
  跳过的注入消息）与 StoryForge 会话的生命周期事件；`/api/storyforge/debug/calls` 另含每次
  调用末尾几条消息的来源结构。

## 已知限制

- StoryForge 会话里的模型选择器被换成只读的「StoryForge 管线」。`/model` 命令仍能改选择：
  请求照样被钉回管线，但 DSH 会每轮在会话历史里追加一条换模型通知，并改掉全局默认模型。
- 一轮写作与后处理串行：上一轮的后处理 + 自动接受没结束，下一轮会等它。多个 StoryForge
  会话共用一个 sidecar，轮到哪个会话就把它的存档设为活跃。
- 自动接受：质量门有 Error 被拦时强制接受（Turn 标 Degraded），保证会话里显示的正文与存档
  一致。
- 自动路由建议升到高成本群像档时直接报错（桌面版此时弹确认），需要到首页把「生成模式」
  显式选成群像后重发。
- 已自动接受的轮次不能重写：DSH 的重试 / 重新生成 / 编辑历史消息会让会话历史与存档错位，
  sidecar 发现两边轮数不一致时拒绝执行并说明（`out_of_sync`），不会悄悄多写一轮。想换走向
  请从最后一条回复分叉；从更早的回复分叉同样会被拒绝（生产 `fork_campaign` 只支持从已采纳
  的末尾分支）。
- sidecar 崩溃或被杀：进行中的一轮报错，下一次请求自动重新拉起，启动恢复清掉中断的 Turn 后
  照常继续；DSH 重启后会话与存档的映射从 `sessions.json` 恢复。
- sidecar 没有 WebView，卡内 MVU 脚本不执行（MVU 翻译产物的原生路径照常生效）。
- 补丁叠加的插件不出现在插件管理页，不能热停用。去掉补丁重启后，StoryForge 会话变为只读
  （发送报 `Unknown agent preset: storyforge`），历史不受影响；带回补丁即恢复。

## PoC 要回答的问题

1. StoryForge 会话能否与普通 DSH 会话完全隔离（专属预设无工具、模型被钉住、视图与面板只在
   StoryForge 会话出现）。
2. StoryForge 管线能否完全通过 DSH 的模型运行，StoryForge 自身不持有任何 API key。
3. StoryForge 会话的创建、开场、分叉、归档、重启能否与 Campaign 保持一致。

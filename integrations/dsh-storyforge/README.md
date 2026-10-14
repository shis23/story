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
| `lib/index.js` | 宿主半边：注册 provider `storyforge`（真实管线适配器，或假管线）、可选的调试回显 provider、`/api/storyforge/*` 受保护路由（新建故事、角色卡列表与导入、底层模型选择与测试连接、sidecar 状态、卡片 HTML 与卡片界面取回 `/display`、净化库 `/vendor/purify.js`、卡片界面 `/shell/*`（挂载、壳文档、宿主代取、Mvu 只读数据、界面状态、确认后的变量写入、开场选择、世界书读写）与桌面模块打包 `/vendor/card-shell.js`、卡内状态 `/mvu`（读取 / 分析 / 应用变量定义 / 交互按钮）、调试） |
| `lib/bridge.js` | sidecar 管理：懒启动、按行收发 JSON-RPC、执行 sidecar 发来的 `llm.stream`、转发管线事件 |
| `lib/agent.js` | StoryForge 预设里的 agent 作用域插件：每次模型请求钉到 `storyforge/pipeline`；记录 agent 创建 / 分叉 |
| `lib/client.js` | 浏览器半边：左侧栏「StoryForge」首页（新建故事、管线设置、我的故事、最近的收尾、底层模型调用、折叠的调试信息）；StoryForge 会话里的视图标签、右栏面板（故事信息 + 卡内状态：状态行、交互按钮）、会话头按钮、停靠条（兼登记本会话的输入框，交互按钮经它发下一轮提示）、模型锁、每轮回复下方的卡片 HTML 与卡片界面 iframe（`conversation.chat.turnTail` 槽；界面的桥请求、变量写入确认条）。颜色与圆角取 DSH 主题变量，跟随亮 / 暗主题 |
| `lib/html.js` | 卡片 HTML 块切分（与桌面 `splitRawHtmlBlocks` 同口径）：DSH 正文去掉完整块与卡片界面、块数与界面数写进 replayState，`/display` 返回的块附文字版 |
| `lib/shell.js` | 卡片界面（卡壳）服务端：按卡清单判定信任、拼壳文档（注入桌面的桥接 / 兼容层脚本，模块转 blob 运行、常用库从 CDN 预加载）、一次性文档路径与 CSP 响应头、白名单宿主代取（磁盘缓存）、壳自己的界面状态（写回存档，按存档串行合并） |
| `lib/vendor/card-shell.mjs` | 生成文件：`shell/entry.mjs` 列出的桌面前端模块（卡壳切分与信任判定、桥接脚本、兼容层、ST API 宿主处理器、变量桶、Mvu 变量树、开场种子与开场选择解析、世界书条目映射、开场界面定高）经 esbuild 打成一个 ES 模块；服务端直接 import，浏览器半边经 `/vendor/card-shell.js` 加载。桌面源码改了要重新生成：`node integrations/dsh-storyforge/scripts/build-vendor.mjs`（用 `frontend/node_modules` 里的 esbuild） |
| `lib/vendor/purify.es.mjs` | DOMPurify 3.4.12（上游双许可 Apache-2.0 / MPL-2.0；同目录 `DOMPURIFY-LICENSE` 是 npm 包自带的许可全文），取自 `frontend/node_modules`，只去掉了 source map 注释行；随插件分发，浏览器半边按需加载 |
| `lib/mvu.js` | 卡内状态（MVU 原生渲染）：桌面 `mvuKey` / `mvuStatusBarModel` / `campaignMvuStatusBar` / `mvuInteractions` / `shellVariableOutbox` 的逻辑逐字移植（键记法、状态行取值、分节、交互解释、写入作用域），加上面板数据与交互计划的装配 |
| `lib/roles.js` | 按系统提示词里的身份句给每次底层调用标管线角色（导演、角色 · 名字、执笔、后处理……），并归入可单独指定模型的五组 |
| `lib/shared.js` | 常量与进程内共享状态（生命周期事件、分叉回调） |
| `lib/cards.js` | 角色卡：从 PNG（`ccv3` / `chara`）或 JSON 读卡名；导入时的文件名清洗与同名编号 |
| `lib/text.js` | 面向用户的文字：会话标题按 DSH 的 80 字节上限预先截断、开场白 HTML 转可读文字、底层模型错误转成可操作的提示 |
| `storyforge.patch.yml` | bundle 补丁（`package.json` 的 `dsh.bundle.patch`，装插件后每次启动自动叠加）：激活宿主插件 + 新增 `storyforge` 预设（只有 persona，无工具） |
| `test/host-smoke.test.mjs` | 宿主半边冒烟：假 DSH ctx + 假底层模型驱动真实 sidecar 跑一轮；角色卡导入；卡内状态的分析 / 应用 / 交互按钮；卡片界面的尾部数据、壳文档与响应头、未登记界面的确认、宿主代取、界面状态（含卸载重启后仍在）与变量写入；开场界面的开场种子、世界书读写、选开场（伪造会话被拒）与写过一轮后关闭；按角色指定模型（设置校验、按组测试连接、每次调用走本组模型） |
| `test/helpers.test.mjs` | 卡名解析、文件名清洗、标题截断、开场白转换、错误提示 |
| `test/roles.test.mjs` | 管线角色识别与分组；身份句逐条核对 Rust 提示词源码（改了措辞先红，不静默退回底层模型） |
| `test/html.test.mjs` | 块切分与桌面实现对拍、正文去块与兜底 |
| `test/mvu.test.mjs` | 卡内状态各函数与桌面实现对拍、面板数据、交互写入目标 |
| `test/shell.test.mjs` | 卡片界面：信任判定、壳文档注入顺序与 CSP、模块改写、模块前言与开场种子与桌面逐字相同、开场态判定与开场白指纹、一次性文档淘汰、宿主代取（白名单 / IP / 重定向 / 大小 / 缓存）、界面状态桶 |
| `test/vendor.test.mjs` | `lib/vendor/card-shell.mjs` 与桌面源码同步（重新打包逐字比对；没装 `frontend` 依赖时跳过） |

插件配置（`storyforge.patch.yml` 里 `config` 下，均可省略；要改就在 profile 自己的补丁里覆盖）：

| 键 | 默认 | 说明 |
|---|---|---|
| `dataDir` | `$DSH_HOME/storyforge` | 会话映射（含已写轮数与最近一次收尾）、设置、收尾报告（`settles.json`）、日志 |
| `cardsDir` | `<dataDir>/cards` | 「新建故事」可选的角色卡（`.png` / `.json`）；首页「导入角色卡…」也存到这里 |
| `bridgeBin` | 仓库 `target/debug/storyforge-dsh-bridge(.exe)` | sidecar 可执行文件 |
| `bridgeDataDir` | `<dataDir>/bridge-data` | StoryForge 存储（JSON 后端），与桌面版数据完全分开 |
| `fakePipeline` | `false` | 用假管线代替 sidecar |
| `debugEcho` | `false` | 额外注册调试回显 provider（仅调试：无凭据的临时实例里模拟普通会话） |

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

2. 安装插件（二选一）。插件在 `package.json` 里声明了 `dsh.bundle.patch`，安装后 DSH 把它加进
   profile 的 bundle 列表，以后每次启动自动叠加 `storyforge.patch.yml`，**不需要 `--patch`**：
   - 界面：启动后进「插件 → 添加插件」，填本地目录
     `C:/Users/Predator/ZCodeProject/storyforge/integrations/dsh-storyforge`，点「安装」，再点「立即启用」
     （不用重启，左侧栏随即出现「StoryForge」）；
   - 命令行（以 `link:` 依赖挂上，改源码不用重装）：

     ```bash
     DSH_HOME="C:/Users/Predator/dsh-poc" dsh plugin --profile sfpoc add "link:C:/Users/Predator/ZCodeProject/storyforge/integrations/dsh-storyforge"
     ```

3. 构建 sidecar：

   ```bash
   cargo build -p storyforge-dsh-bridge
   ```

4. 启动（端口由系统分配，不自动开浏览器）：

   ```bash
   cd /c/Users/Predator/dsh-poc/workspace && DSH_HOME="C:/Users/Predator/dsh-poc" dsh --profile sfpoc --port 0 --no-open
   ```

5. 打开输出里的 `dsh web: http://127.0.0.1:<端口>/?token=…` 链接。先在 DSH 设置里配好
   至少一个模型服务商（凭据由你在界面里填，插件不读任何凭据文件），再到左侧栏
   「StoryForge」首页：
   - 选「底层模型」：没单独指定模型的角色都用它。点「测试连接」发一条很短的请求，确认
     密钥与网络可用（没配密钥时直接提示去哪里配）；
   - 可选「按角色指定模型」：导演、角色表演、成文（执笔 / 编剧 / 对手戏定稿）、后台收尾
     （摘要 / 后处理 / 纪要压缩）、导入与分析（角色识别 / 卡内状态分析）各自可以换成别的
     模型，比如导演和后台收尾用快模型、成文用强模型；每组可以单独测试连接。插件按系统提示词
     里的身份句判断一次调用属于哪组，认不出的调用（如消息归档）用底层模型；首页「底层模型
     调用」表的「模型」列是每次实际用的模型。工具协议对所有模型生效，选的模型都要支持它；
   - 「生成模式」默认自动路由；也可以显式选续写 / 双人戏 / 群像（并行或顺序子 Agent）。
     「工具协议」默认原生工具调用，模型不支持 function calling 时选「文本降级」（与
     StoryForge 连接的 TextFallback 同一套实现）；
   - 「导入角色卡…」选一张 SillyTavern 卡（`.png` / `.json`，20 MB 以内；只收能读出卡名的
     卡，同名不覆盖）。下拉框显示卡名。也可以直接把卡文件放进 `$DSH_HOME/storyforge/cards/`
     再点「刷新」；
   - 点「新建故事」：sidecar 导入卡、跑角色识别（会调用模型）、开档，然后打开新会话（建在
     `stories` 工作区，停在「对话」视图并打开右栏 StoryForge 面板），第一条回复是卡的开场白
     （开场白里的状态栏 HTML 渲染在回复下方，见「已知限制」的卡片 HTML 一条）。角色识别失败时（多半是
     底层模型没配好）留在首页说明原因，可以修好模型后新建，也可以「仍然打开故事」；
   - 在会话里输入行动：思考块显示导演 / 角色 / 执笔的过程，末尾一行是本轮的模式与底层
     调用用量；回复正文是质量门之后的成文。会话里的用量是本轮正文阶段所有底层调用之和；
     后处理在后台继续，结束后自动接受（停靠条与面板显示结果和后处理用量）；
   - 首页「我的故事」按最近活动列出关联存档的故事（已写轮数、角色卡、分叉来源、最近一次
     接受结果），点整行打开；未关联存档的会话默认折叠。「最近的收尾」与故事进度落盘，DSH
     重启后仍在。「底层模型调用」表逐次列出管线发出的调用，「角色」列标出是导演、哪个角色、
     执笔还是后处理；
   - 右栏 StoryForge 面板下半部是「卡内状态」：卡里有 MVU 变量脚本时，点「分析变量规则」（调用一次
     底层模型）把卡的状态栏和按钮翻译成原生面板，再点「应用到角色」把分析出的变量定义写进卡内角色
     （已有角色补默认值）。之后状态行（进度条 / 文字 / 标签 / 图标）随每轮后处理更新；点交互按钮
     按卡的规则改变量，带「下一轮提示」的按钮再替你发一条消息；
   - 从某条回复「分叉」：子会话第一次发送时映射成存档的分支（StoryForge 的 Campaign
     分支），之后两边各写各的。只支持从**最后一条**回复分叉。

   从新会话页的预设下拉里直接选「StoryForge」建的会话没有关联存档，发送会报错。

调试无凭据的临时实例时，可以在 profile 的补丁里给 `storyforge` 加 `debugEcho: true`：多注册一个
本地回显 provider `sf-debug-echo`（模型选择器里的「调试回显」），用来模拟普通会话。它回显本轮
用户输入并报告收到的工具数，不发任何外部请求。

## 开发循环

- 改 `lib/client.js`：刷新页面即可生效。
- 改 `lib/index.js` / `lib/bridge.js` / `lib/agent.js` / `lib/cards.js` / `lib/text.js` / `lib/html.js` / 补丁：重启实例。
- 改 Rust：`cargo build -p storyforge-dsh-bridge` 后重启实例（sidecar 随插件拉起）。
- 测试：`cargo test -p storyforge-dsh-bridge`（含拉起 sidecar 进程、Mock 模型跑两轮的端到端
  测试）；`node --test integrations/dsh-storyforge/test/*.test.mjs`
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
  照常继续；DSH 重启后会话与存档的映射从 `sessions.json` 恢复。例外：正文已经显示、后台收尾
  （后处理 + 自动接受）还没结束时崩溃，这一轮不会被接受，存档比会话少一轮，之后发送会报
  `out_of_sync`。目前没有自动补救：从上一条已接受的回复分叉继续，或新建故事。
- 停止：成文中停止会中止这一轮；排队时（思考块第一行是「【排队】」）停止只放弃这一轮，上一轮的
  收尾照常完成。
- 卡片 HTML 只做了净化渲染这一层：开场白和每轮回复里**完整闭合**的顶层 `details` / `div` /
  `table` / `ul` / `ol` / `blockquote` 块（与桌面版 `splitRawHtmlBlocks` 同口径）从正文切走，
  经 DOMPurify（配置同桌面版）净化后渲染在这一轮回复下方。样式关在 Shadow DOM 里，外层
  `contain: paint`，卡里的 `position: fixed` 一类样式出不了框。这些块里的脚本、事件属性、表单、
  iframe 一律去掉；要跑脚本的卡片界面另走 sandbox iframe（见「卡片界面」一条），MVU 变量规则另走
  右栏的卡内状态面板。远程图片照常加载，链接一律新窗口打开（`rel="noopener noreferrer"`）。
  存档里的原文不变。
  - 切块前先整段去掉完整的 `<script>` 元素：MVU 状态栏一类卡会在脚本的字符串里拼 `<div>`，不去掉
    会被当成顶层块，尾部渲染出 `' + s.体力 + '` 一类碎片。这类卡的尾部通常只剩静态占位（如
    「加载中…」），真正的状态看右栏卡内状态面板。
  - 渲染内容取自存档的展示文本（sidecar `conversation.display`：按节点套卡的展示正则），不缓存。
    展示正则带深度限制（`maxDepth`）的卡，旧轮次随对话变长退回原文，状态栏只在最近几轮显示，
    与桌面版一致；所以每多一轮，已挂载的回复尾部整批重取一次。
  - 正文里没有完整块时（未闭合的块、`<data_block>` 一类数据标签）照旧整段转文字列表（表格每行
    一项，单元格用 ` | ` 分隔）。整条回复只有块时正文退回整段文字版、尾部不再重复（DSH 不收空
    回复，空回复会让下一轮错位）。整条回复只有卡片界面时正文是一句占位「（卡片界面，显示在这条
    回复下方。）」。DOMPurify 加载失败或净化后为空时显示文字版；取回失败显示一行错误和「重试」。
  - 分叉出的子会话里，继承来的回复按节点位置在分支存档里取（分支会重写节点 id）。源对话还在时
    比对正文，对不上就不渲染。
  - 本功能之前写下的回复没有渲染引用，不显示尾部，正文仍是当时的文字版。
  - 质量门的格式泄漏检查只看 `<div>` / `<span>` / `<p>` 与代码块围栏：开场白里用过的算这张卡
    的既定格式，模型每轮照写时放行；开场白没用过的交给 autofix 改写。`<think>` 一律算泄漏，
    其余标签（如 `<details>`、`<table>`）不检查。
- 卡片界面（卡壳，与桌面版消息里的卡壳同口径）：回复里的 `$('body').load(url)` 胶水和带 `<script>`
  的内联 HTML 文档（桌面 `segmentShellContent` 切分）从正文切走，在这一轮回复下方的 iframe 里运行。
  - 隔离：iframe 只给 `sandbox="allow-scripts"`，永远不给 `allow-same-origin`，界面是不透明源，
    读不到 DSH 的 cookie、页面和接口。壳文档由插件服务端拼（浏览器只说挂哪条回复里的哪个界面，
    服务端从存档重新取展示文本并复核），登记成一次性路径 `/shell/doc?t=…`（1 小时、最多 64 份），
    响应头带 CSP（含 `sandbox allow-scripts`），网络类指令只放行主机白名单（与桌面默认白名单同一
    常量：jsdelivr、cdnjs、Google Fonts、catbox、imgbb、postimg、GitHub、Gitee）。
  - 信任：只有卡清单里登记过的 `.load` URL、命中卡正则的内联文档自动挂载（与桌面相同）；其余显示
    确认卡，点「信任并挂载本次」才挂，刷新页面后重新确认。
  - 每轮都带状态栏的卡只展开最新一轮的界面，较早的默认收起、点「展开」再挂。
  - 开场界面（首页、自定义开局；卡清单里登记过的才算）挂在开场白这一轮下方，按窗口高度定高（与
    桌面相同的规则）。只在开局前可用：会话已关联存档、对话里只有开场白一个节点；写过一轮后（新的
    一轮只有文字也算，不用刷新页面）只留一行说明，卡的展示正则限定了深度的（如命定之诗）整个不再
    显示，服务端也拒绝再挂。界面里切换开场（卡调 `reloadCurrentChat`）时按卡的问候语解析选择，
    经 sidecar 改写存档里的开场白（桌面 `apply_campaign_opening` 同一入口）；只认插件挂过的开场
    界面的桥会话，别的界面或伪造的请求被拒。DSH 正文里还是开局时的那份开场白，存档里的开场白变了
    时尾部最前面补一段新开场的文字（新开场只有界面时不补）。不在清单里的开场界面照常要确认，当
    普通界面挂，不能改开场。
  - 运行环境：jQuery 3.7.1、lodash 4.17.21、Vue 3.5.13、Vue Router 4.5.1、EJS 3.1.10 从 cdnjs /
    jsdelivr 预加载，Zod 4 异步加载，所以界面要能访问这些 CDN。界面是不透明源，读真正的
    `localStorage` / `sessionStorage` 会报错，所以换成每份文档自己的内存版（不与 DSH 共享，重新
    加载后清空）。卡里的内联模块脚本在 iframe 里转 blob 运行，相对
    import 按页面地址改成绝对地址。`$.fn.load` / `$.getScript` 与卡的远程资源经插件服务端代取：
    只取白名单主机、不取 IP、手动跟随重定向且每一跳复核、单个 40 MB、30 秒超时；取到的资源缓存在
    `<dataDir>/card-shell-cache`，首取即冻结（与桌面相同，想更新就删掉这个目录）。
  - 数据：`Mvu` 读到的是存档变量树（只读）。界面自己用 `getVariables` / `insertOrAssignVariables`
    存的状态写回存档（Campaign 变量 `__storyforge_card_shell_variables`，与桌面版同一个键、同一个
    后端入口），DSH 重启后还在；同一存档的写入串行合并，界面不等落盘就得到回复（写作进行中落盘
    排在这一轮收尾之后，插件卸载时最多等 2 秒）。落盘失败只记日志，下一次写入带整份重试。还没
    关联存档的会话只放插件内存。界面请求写存档变量时先在界面下方列成提案，点「应用」才写，
    写入目标与键名守卫同卡内状态面板的交互按钮；写入失败时提案保留。
  - 世界书：界面经酒馆助手 `getWorldbook` / `updateWorldbookWith` 读本局世界书的条目名与启用状态、
    按条目名切换启用（只认本局书名 `storyforge:campaign:<存档 id>`，不读正文、不改其它字段）；同一
    存档的读改写串行，与桌面同口径。写作进行中切换要等这一轮收尾。
  - 发消息：界面用 `createChatMessages` 添加的用户消息只填进本会话的输入框（多条用空行隔开），
    界面下方提示「看过后点发送」，由用户自己发出；`/trigger` 不做事。输入框里已有别的草稿、正在
    写作或输入框不在当前页面时不填，错误回给界面。只接受用户消息，添加 AI 消息报错。命定之诗的
    自定义开局点「踏上旅程」后，生成的角色设定出现在输入框里。桌面版同口径（填进写作输入框）。
  - 不支持：其它需要 StoryForge 后端的 SillyTavern / 酒馆助手接口（生成、改写或删除消息等），
    界面里调用会报错。
  - 本功能之前写下的回复不显示卡片界面。内置浏览器面板一类会拦截子 frame 的环境里 iframe 是空白。
- 卡内状态面板（MVU 原生渲染，与桌面版写作页的状态面板同口径）：
  - 分析是手动的（桌面版同样不自动跑），每次调用一次底层模型，大卡要几十秒。分析结果存在 sidecar
    自己的数据目录里，与桌面版不共用；每个新故事都重新导入角色卡，所以每个故事要各分析一次，
    分叉出的子会话共用来源故事的分析。模型调用失败时分析器会退回只有变量字段的结果：面板提示
    「没拿到状态栏规则」并附上底层错误，可以修好模型后「重新分析」。
  - 分析器只把 `extensions.tavern_helper.scripts`、`extensions.mvu.script`、卡资源 JS 当作卡内脚本，
    卡里也没有 `mvu.initvar` / `stat_data` 一类变量数据时直接按纯数据卡处理、不调用模型（面板提示
    「只得到变量字段」）。只把脚本写在正则替换串里的卡属于这种情况（桌面版同样如此）。
  - 「应用到角色」改的是这张卡在 sidecar 里的角色定义（本故事与它的分叉共用同一张卡），已有角色只补缺的变量、不覆盖现值。
  - 交互按钮：浏览器只报按钮位置和文字，写哪些变量、写什么值由插件服务端按最新变量重新计算
    （面板过期时拒绝并刷新）。写入目标与桌面版相同：卡绑定的角色恰好一个时写到它身上，否则整局
    只有一个角色时写到它身上，都不是就写到存档全局变量。键名守卫与桌面写变量命令是同一个入口
    （`__` 内部段、字符集、长度）。写变量、分析、应用都排在上一轮后台收尾之后，写作或收尾进行中
    按钮禁用。有写入失败时不发下一轮提示；输入框里有草稿、正忙或不在当前页面时也不发，提示文字
    留在面板里自己发。按钮规则里解释不了的取值表达式（桌面版同口径：只认 `+N` / `-N`、`变量 ± N`、
    JSON 字面量与纯文字，`min(100, 体力 + 20)` 一类带函数的不认）不执行，面板说明跳过了哪些。
  - 需要卡内脚本的部分（分析结果里的兜底片段）只列出说明，不执行。
- 插件管理页可以热停用 / 启用（不用重启）。停用后 StoryForge 首页消失、sidecar 退出，StoryForge
  会话从会话列表隐藏，数据保留在磁盘上；重新启用后刷新一次页面，会话回到列表。
- DSH 会话标题上限 80 字节，分叉时在末尾追加 ` (1)`。插件给 DSH 的标题预先截到 72 字节并补
  省略号，免得分叉序号被截成半个括号。

## PoC 要回答的问题

1. StoryForge 会话能否与普通 DSH 会话完全隔离（专属预设无工具、模型被钉住、视图与面板只在
   StoryForge 会话出现）。
2. StoryForge 管线能否完全通过 DSH 的模型运行，StoryForge 自身不持有任何 API key。
3. StoryForge 会话的创建、开场、分叉、归档、重启能否与 Campaign 保持一致。

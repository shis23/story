# StoryForge 用户指南（发布候选）

> 状态：2026-07-07 发布候选。本文面向首次使用者，覆盖使用前准备、导入 ST PNG/JSON、创建 Campaign、写第一轮、查看状态、失败排障、导出 Campaign bundle、兼容/降级边界、本地数据与备份。界面入口名称可能随候选版本微调；不确定处以当前界面中含义相同的 Campaign、导入、导出、排障相关入口为准。

## 1. 使用前准备

开始前先准备好这些材料，避免把测试数据、正式数据和凭据混在一起：

- ST PNG/JSON：至少准备一张可用于测试的 SillyTavern PNG 或 JSON。建议先用小卡跑通，再导入真实复杂卡；真实卡原文件应单独保留，不要只依赖导入后的副本。
- 可用 LLM 连接：确认已经配置一个可用的 LLM 连接；如果流程需要 embedder，也先确认 embedder 配置可用。真实 LLM 会受网络、模型状态和费用影响，发布候选测试时应记录模型、endpoint 和失败时间。
- 测试/正式数据目录：首次验证、坏卡复现或升级试跑应尽量使用空白或临时测试数据目录；正式写作继续使用应用管理的数据目录。不要把测试目录当成长期正式库，也不要把正式数据拿来反复破坏性试验。
- API key 安全：不要把真实 API key 写进文档、issue、截图、日志摘要、排障说明或示例配置。连接和 embedder 配置文件中只应出现 `storyforge-secret:v1:*` 这类 SecretRef 引用，真实 key 由系统凭据库存储。
- 升级前备份：如果已有重要 Campaign，升级或换版本前先导出 Campaign bundle，并保留一份排障 bundle，便于失败后定位数据目录和日志。

## 2. 核心概念

StoryForge 的写作主线围绕 Campaign，而不是单张角色卡。

- ST 卡：从 SillyTavern PNG/JSON 导入的角色或多角色资料。StoryForge 会尽量保留原始字段、extensions 和 raw JSON。
- Character Definition：从角色卡抽取出的角色定义，可以被多个 Campaign 复用。
- Character Instance：某个 Campaign 里的角色实例。即使两个角色显示名相同，也应通过 instance id 隔离知识和变量。
- Campaign：一次故事运行的容器，保存角色实例、对话树、摘要、知识、变量、任务、Meta patch 和 MVU 相关状态。
- Pipeline trace：每轮写作中 Director、Subagent、Editor、Postprocess 的输入输出摘要，用来解释和排障。
- Campaign bundle：用于备份、迁移、复现问题或分享 StoryForge 内部 Campaign 状态的导出包，内容以 StoryForge JSON 为主。
- 排障 bundle：用于反馈失败的诊断包，通常包含 app/platform 摘要、数据目录/日志/会话路径摘要和关键 store 摘要；它不应包含真实 API key。
- StoryForge JSON：StoryForge 内部导出格式，不等同于原始 ST 卡。需要保留完整 Campaign 状态时，优先导出 Campaign bundle。

## 3. 导入 ST PNG/JSON

1. 打开 StoryForge。
2. 在角色、导入或类似入口中选择 ST PNG 或 JSON。
3. 导入完成后检查角色列表是否刷新。
4. 打开角色详情，确认名称、简介、开场白、标签、世界书或 extensions 没有明显丢失。
5. 如果是多角色卡，运行角色抽取或按界面提示创建多个 definitions。

导入失败时，不要在正式数据目录里反复覆盖。先记录卡文件名、文件类型、错误提示和发生时间；如果界面可用，立即导出排障 bundle。需要继续复现时，改用空白或临时测试数据目录。

## 4. 创建 Campaign

1. 从已导入的角色卡创建 Campaign。
2. 选择角色卡；StoryForge 会把可用的 Protagonist/Supporting definitions 加入为 Campaign instances。
3. 给 Campaign 设置可识别的名称。
4. 将它设为 active Campaign。
5. 打开 Campaign 面板，确认 instances、variables、knowledge、tasks、summaries 等标签页可见。

多角色 Campaign 中，尽量避免依赖显示名做判断。两个同名角色应被视为不同 instance，知识和变量也应落到对应的 instance id。

## 5. 写第一轮

1. 确认当前写作入口显示的是 active Campaign。
2. 确认可用 LLM 连接已经选中或处于可用状态。
3. 输入第一轮用户消息或场景指令。
4. 开始生成。
5. 等待正文输出完成。
6. 等待 postprocess 写回完成后，再查看 Campaign 状态。

第一轮成功的最低标准：

- Director 能看到 Campaign 中的角色实例。
- Subagent 按角色生成内容。
- Editor 输出正文。
- Pipeline trace 能显示本轮 Agent 摘要。
- 写作结果落到当前 Campaign，而不是 legacy 单角色路径。

## 6. 查看状态

写作后重点查看这些位置：

- Pipeline：检查 Director 选了哪些角色、Subagent 是否分角色输出、Editor 是否产出正文。
- Summaries：确认本轮摘要是否生成。
- Knowledge：确认知识写入了正确知道者，来源者和 provenance 文案可追踪。
- Variables：确认变量落在 Campaign 或正确角色 instance。
- Tasks：确认任务创建、完成、放弃等状态可见。
- Meta：运行 health check，必要时查看“解释本轮生成”或类似解释入口。

如果状态看起来不对，先按“出问题先做什么”处理，再决定是否使用 Meta patch。Meta patch 应先 preview，再 accept；dismiss 不应改变 Campaign 状态。

## 7. 出问题先做什么

失败处理优先目标是保住现有数据、留下可复现线索，然后再重试。

1. 先停手：不要连续点击生成、导入或覆盖，也不要手工改 store 文件。
2. 记下现场：记录 StoryForge 版本、平台、Campaign 名称、ST 文件名、正在执行的操作、错误文案和发生时间。
3. 导出材料：优先导出排障 bundle；如果 Campaign 还能打开，也导出 Campaign bundle。分享失败信息前检查截图、日志摘要和文档里没有真实 API key。
4. 做轻量检查：确认 LLM 连接可用、网络可用、当前是预期的 active Campaign，连接配置文件只保存 `storyforge-secret:v1:*` 引用。
5. 换测试目录复现：坏卡、升级、迁移或 Android 导入问题，优先在空白或临时测试数据目录复跑，不要拿正式目录反复试错。
6. 重启后复核：重启 StoryForge 后确认 active Campaign、Pipeline、summaries、knowledge、variables、tasks 仍可查看。
7. 再决定修复：需要修复状态时，优先使用界面提供的 Meta preview/accept 流程；不要手工复制凭据或直接覆盖内部数据文件。

反馈问题时，最有用的材料是：操作步骤、平台、版本、ST 文件名、错误提示、排障 bundle、必要时的 Campaign bundle，以及已经打码的截图。

## 8. 导出 Campaign Bundle

Campaign bundle 用于备份、迁移、排障复现或分享 StoryForge 内部 Campaign 状态。

1. 打开目标 Campaign。
2. 在导出相关入口选择 Campaign bundle。
3. 保存生成的 StoryForge JSON。
4. 记录 StoryForge 版本、导出时间和 Campaign 名称。
5. 如需排障，同时导出排障 bundle。

导出后建议做一次冒烟检查：重新导入或打开导出物，确认角色实例、对话、摘要、知识、变量和任务仍在。

Campaign bundle 和排障 bundle 都不应包含真实 API key。连接和 embedder 配置文件应只保存 `storyforge-secret:v1:*` 形式的引用，真实 key 由系统凭据库存储。

## 9. ST 兼容/降级边界

当前发布候选可说明的兼容范围：

- 支持常见 ST V2/V3 PNG/JSON 导入。
- 保留 raw JSON 和未知 extensions，避免导入时丢失原始信息。
- 支持从多角色卡创建 StoryForge Campaign，不强制退化为单角色卡。
- 支持世界书 Constant/Both 稳定注入，Selective/Both 按关键词和本轮意图触发。
- 支持 MVU schema preview、基础变量 apply 和状态栏关键变量渲染。
- 支持 Campaign bundle（StoryForge JSON）导出；ST 卡 PNG 和共享 lorebook 导出用于兼容场景。

需要明确降级或尚未承诺的范围：

- 不承诺 ST 99 事件全集、全部 prompt hooks 或冷门事件全量兼容；前端已为声明 `ModifyPrompt` 的插件提供常驻隐藏 `PluginHost` hook host，支持 host→iframe 可等待 hook 请求/响应，并会在写作前触发 `GENERATE_BEFORE_COMBINE_PROMPTS` 与 `CHAT_COMPLETION_PROMPT_READY` 改写写作入参；写作/重 roll 的最终 LLM messages 级 hook 也已接入，插件错误或超时会回退原 messages；普通事件 feed 已按 `event_subscriptions` 和 `ReadMemory` 做订阅/正文脱敏；prompt hook 已有基础脱敏审计记录；全量 ST 事件、冷门 Slash/TavernHelper 语义和完整审计 UI/导出仍在推进中。
- Regex Slash placement 3 的最小 `/` 前缀 hook 已接入；插件桥已提供常用 Slash 命令注册/触发 fallback，并能解析基础 raw/named/unnamed 参数。
- PluginHost 已支持 per-slot 斜杠/状态栏挂载；复杂 JS 状态栏仍需真实卡回归，JS 失败时应以降级提示和原生状态展示为准。
- JSR/ST/TavernHelper API shim 只覆盖常用子集，依赖冷门 API、pipe 语义或完整 prompt hooks 的重 DOM 卡可能跳过 JS 更新。
- 传话链当前依赖文本匹配，不是完整语义级追踪。
- private/封口知识是文本匹配级门禁，不等同完整安全边界；真实 LLM 对抗仍需发布候选实跑。

这些边界用于降低误解：发布候选可以跑主流程和常见兼容场景，但不要把它描述成正式全量 ST 运行时。

## 10. 本地数据与备份

StoryForge 以本地数据为主。应用不应被视为云同步或唯一备份来源；重要 Campaign 需要用户主动导出 Campaign bundle。

桌面端：

- 应用数据目录由 StoryForge/Tauri 管理。正常使用时不要在应用运行中手工移动、复制或覆盖内部 store 文件。
- 测试版、开发版或复现失败时，尽量使用单独的测试数据目录；正式写作使用稳定的正式数据目录。
- 排障 bundle 会记录 data、log、conversation 等路径摘要，帮助定位问题，但不应导出真实 API key。
- 升级前建议导出 Campaign bundle；遇到失败时再保留一份排障 bundle。

Android：

- 数据位于 Android 应用沙盒中，不能按桌面文件夹方式手工管理。
- Android 的系统文件选择器导入、save/share sheet 导出、keyring 读写删和升级恢复必须真机验收；构建通过或模拟器通过不能替代真机结果。
- 升级或重装前优先通过界面导出 Campaign bundle；排障时同时导出排障 bundle。

备份建议：

- 重要 Campaign 每次阶段性完成后导出 Campaign bundle（StoryForge JSON）。
- 发布候选测试前后都保留一份排障 bundle，便于比对失败前后的数据目录和日志摘要。
- 不要手工复制或公开含真实凭据的系统文件；也不要把 `connections.json`、`embed.json` 当作凭据备份。
- 如果迁移失败，停止写入并保留旧目录；不要用新空目录覆盖旧数据。

## 11. 首次使用检查

完成第一局 Campaign 前，逐项确认：

- 已准备至少一张 ST PNG 或 JSON。
- 已配置可用 LLM 连接，并确认不会泄露 API key。
- 已区分测试数据目录和正式数据目录。
- 已导入角色卡并检查角色详情。
- 已创建 Campaign，并设为 active Campaign。
- 已写完第一轮。
- Pipeline trace 可见。
- Summaries、Knowledge、Variables、Tasks 至少能打开查看。
- 可导出 Campaign bundle。
- 失败时知道先导出排障 bundle，再记录步骤和错误提示。

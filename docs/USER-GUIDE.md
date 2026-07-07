# StoryForge 用户指南草案

> 状态：2026-07-07 草案。本文面向首次使用者，覆盖导入 ST 卡、创建 Campaign、写第一轮、查看状态、导出 Campaign bundle、ST 兼容范围、本地数据与备份。

## 1. 核心概念

StoryForge 的写作主线围绕 Campaign，而不是单张角色卡。

- ST 卡：从 SillyTavern PNG/JSON 导入的角色或多角色资料。StoryForge 会尽量保留原始字段、extensions 和 raw JSON。
- Character Definition：从角色卡抽取出的角色定义，可以被多个 Campaign 复用。
- Character Instance：某个 Campaign 里的角色实例。即使两个角色显示名相同，也应通过 instance id 隔离知识和变量。
- Campaign：一次故事运行的容器，保存角色实例、对话树、摘要、知识、变量、任务、Meta patch 和 MVU 相关状态。
- Pipeline trace：每轮写作中 Director、Subagent、Editor、Postprocess 的输入输出摘要，用来解释和排障。

## 2. 导入 ST 卡

1. 打开 StoryForge。
2. 在角色或导入入口选择 ST PNG 或 JSON。
3. 导入完成后检查角色列表是否刷新。
4. 打开角色详情，确认名称、简介、开场白、标签、世界书或 extensions 没有明显丢失。
5. 如果是多角色卡，运行角色抽取或按界面提示创建多个 definitions。

导入失败时，不要反复覆盖原数据。先记录卡文件名、错误提示，并导出排障 bundle。

## 3. 创建 Campaign

1. 从已导入的角色卡创建 Campaign。
2. 选择角色卡；StoryForge 会自动把可用的 Protagonist/Supporting definitions 加入为 Campaign instances。
3. 给 Campaign 设置可识别的名称。
4. 将它设为 active Campaign。
5. 打开 Campaign 面板，确认 instances、variables、knowledge、tasks、summaries 等标签页可见。

多角色 Campaign 中，尽量避免依赖显示名做判断。两个同名角色应被视为不同 instance。

## 4. 写第一轮

1. 确认当前写作入口显示的是 active Campaign。
2. 输入第一轮用户消息或场景指令。
3. 开始生成。
4. 等待正文输出完成。
5. 等待 postprocess 写回完成后，再查看 Campaign 状态。

第一轮成功的最低标准：

- Director 能看到 Campaign 中的角色实例。
- Subagent 按角色生成内容。
- Editor 输出正文。
- Pipeline trace 能显示本轮 Agent 摘要。
- 写作结果落到当前 Campaign，而不是 legacy 单角色路径。

## 5. 查看状态

写作后重点查看这些位置：

- Pipeline：检查 Director 选了哪些角色、Subagent 是否分角色输出、Editor 是否产出正文。
- Summaries：确认本轮摘要是否生成。
- Knowledge：确认知识写入了正确知道者，来源者和 provenance 文案可追踪。
- Variables：确认变量落在 Campaign 或正确角色 instance。
- Tasks：确认任务创建、完成、放弃等状态可见。
- Meta：运行 health check，必要时查看“解释本轮生成”。

如果状态看起来不对，先导出排障 bundle，再决定是否用 Meta patch。Meta patch 应先 preview，再 accept；dismiss 不应改变 Campaign 状态。

## 6. 导出 Campaign Bundle

Campaign bundle 用于备份、迁移、排障或分享 StoryForge 内部 Campaign 状态。

1. 打开目标 Campaign。
2. 选择导出 Campaign bundle。
3. 保存生成的 JSON bundle。
4. 记录 StoryForge 版本、导出时间和 Campaign 名称。
5. 如需排障，同时导出 log/export bundle。

导出后建议做一次冒烟检查：重新导入或打开导出物，确认角色实例、对话、摘要、知识、变量和任务仍在。

排障 bundle 和 Campaign bundle 不应包含真实 API key。连接和 embedder 配置文件应只保存 `storyforge-secret:v1:*` 形式的引用，真实 key 由系统凭据库存储。

## 7. ST 兼容范围

当前可作为本轮发布说明的兼容范围：

- 支持常见 ST V2/V3 PNG/JSON 导入。
- 保留 raw JSON 和未知 extensions，避免导入时丢失原始信息。
- 支持从多角色卡创建 StoryForge Campaign，不强制退化为单角色卡。
- 支持世界书 Constant/Both 稳定注入，Selective/Both 按关键词和本轮意图触发。
- 支持 MVU schema preview、基础变量 apply 和状态栏关键变量渲染。
- 支持 StoryForge JSON bundle 导出；ST 卡 PNG 和共享 lorebook 导出用于兼容场景。

需要明确降级或未完成的范围：

- Regex Slash placement 3 的最小 `/` 前缀 hook 已接入；完整 Slash 命令注册、参数管道和 PluginHost/JS 斜杠运行时仍待补。
- 完整 PluginHost/JS 状态栏运行时仍待补。
- ST 99 事件全集和 prompt hooks 不能承诺全量兼容。
- JSR/ST API shim 只覆盖常用子集，依赖冷门 API 的重 DOM 卡可能跳过 JS 更新。
- 传话链当前依赖文本匹配，不是完整语义级追踪。
- private/封口知识是文本匹配级门禁，不等同完整安全边界；真实 LLM 对抗仍需发布候选实跑。

## 8. 本地数据与备份

StoryForge 以本地数据为主。发布候选需要同时验证桌面端和 Android 端的数据目录策略。

桌面端：

- 应用数据目录由 StoryForge/Tauri 管理。
- 排障 bundle 会记录 data、log、conversation 等路径摘要。
- 升级前建议导出 Campaign bundle，并保留一份排障 bundle。

Android：

- 数据位于 Android 应用沙盒中。
- 真机仍需验证系统文件选择器导入、save/share sheet 导出、keyring 读写删和升级恢复。
- 升级或重装前优先导出 Campaign bundle；排障时同时导出 log/export bundle。

备份建议：

- 重要 Campaign 每次阶段性完成后导出 StoryForge JSON bundle。
- 发布候选测试前后都保留一份排障 bundle。
- 不要手工复制或公开含真实凭据的系统文件。
- 如果迁移失败，停止写入并保留旧目录；不要用新空目录覆盖旧数据。

## 9. 首次使用检查

完成第一局 Campaign 前，逐项确认：

- 已导入至少一张 ST PNG 或 JSON。
- 已创建 Campaign，并设为 active Campaign。
- 已写完第一轮。
- Pipeline trace 可见。
- Summaries、Knowledge、Variables、Tasks 至少能打开查看。
- 可导出 Campaign bundle。
- 失败时知道从哪里导出排障 bundle。

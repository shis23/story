# 项目意图

StoryForge 的核心意图是构建一个可持续运行的 AI 故事存档系统。

它不是：

- 普通 LLM 聊天 UI。
- 只导入 ST 卡并调用模型的轻壳。
- 单轮 prompt 拼接工具。
- 无状态的角色扮演页面。

它应该是：

- 以 Campaign 为中心的长线写作系统。
- 兼容 ST 角色卡，但有自己的运行时模型。
- 多 Agent 分工明确的写作流水线。
- 能把正文反写为变量、知识、任务和摘要的状态机。
- 能解释和修复自身数据的创作工作台。

## 关键决策

1. Android-first，但桌面端保留为开发和调试环境。
2. ST 卡必须兼容导入，原始 JSON 和 extensions 不能轻易丢弃。
3. 多角色卡要拆成 `CharacterCard` + `CharacterDefinition`，运行时再实例化为 `CharacterInstance`。
4. Campaign 是存档和隔离边界。
5. 写作流水线应围绕 CampaignRuntimeContext，而不是直接读 Tauri store。
6. Agent 输出进入持久化前必须结构化校验。
7. Meta Agent 是维护层，不抢 Director/Editor 的写作职责。

## 当前最重要的取舍

不要继续堆外围功能。先让 Campaign 成为主线：

- 导入卡后创建 Campaign。
- 写作使用 Campaign instances。
- 后处理写回 Campaign 状态。
- 下一轮写作读取这些状态。

这个闭环跑通后，插件、MVU、导出、移动端体验和 Meta Agent 都有稳定落点。

# StoryForge 交接说明

> 更新日期：2026-07-08
> 范围：发布验收、文档归档与下一步推进。本文只描述当前状态、验证方式和下一优先级，不替代具体实现计划。

## 当前项目状态

- Campaign 主线已经成为写作运行时真相源：Director、Subagent、Editor、Postprocess 都围绕 Campaign/CharacterInstance 工作。
- Agent Profile 与 Character Extraction 的主计划已完成并归档到 `docs/archive/2026-07-08-completed-plans/`；后续增强继续作为验收和质量任务跟踪，不再把这两个 plan 当作当前入口。
- Meta Agent 维护层已完成基础闭环：health check、解释生成、typed patch、preview/accept/dismiss、MVU schema preview/apply 都已接入；MetaPanel 接受 patch 后的状态流已有前端纯模型测试覆盖。
- ST 导入/导出与 MVU 基础能力已落地：V2/V3 导入保真、raw JSON/extensions 保留、Campaign JSON bundle、ST 卡 PNG/共享 lorebook 导出、MVU 状态栏/schema preview、JS fallback runtime 接入写作流程。
- ST/插件兼容已推进到主生成事件、常见聊天事件别名、`eventSource`/`TavernHelper` 常用 shim、Slash 注册/触发/注销、ModifyPrompt 前端 hook、最终 LLM messages 级 prompt hook 和普通事件订阅/正文脱敏。
- Prompt hook 现在有基础审计记录：前端会记录脱敏后的 hook 请求/响应、插件 ID、事件名、耗时、错误摘要和 payload hash/长度；这不是完整的可筛选 UI/导出审计面板。
- UI smoke runner 已加入：`scripts/run-ui-smoke.ps1` 与 `frontend` 的 `npm run smoke:ui` 会在本机已有 `@playwright/test` 时跑浏览器级冒烟；当前环境缺少 Playwright 时会写 `artifacts/ui-smoke/SKIPPED.txt`，不能替代真实 Tauri 桌面 UI 证据。
- 自动化发布基线已建立：`scripts/verify-release.ps1` 覆盖 secret scan、cargo fmt、workspace clippy/tests、frontend test/build；上次完整非沙箱 release gate 已通过，Vite dynamic/static import warning 仍按既有风险记录。
- Android 仍处于打磨阶段：arm64-v8a debug/release 构建链路已有记录，但真机安装、文件导入、share/save sheet、Android keyring 和长会话稳定性仍需现场验收。
- 真实 LLM 仍需发布候选实跑：确定性 harness 已通过，真实 LLM ignore 用例、真实卡、多轮质量、知识隔离/传播对抗和成本记录仍需补齐。

## 最新提交

- `d4d1ffa test: add ui smoke runner`
- `d72164b feat: audit prompt hook mutations`
- `339d892 fix: align st popup constants`
- `9498146 feat: add st tavern event globals`
- `3e8d781 fix: satisfy release clippy gate`
- 更早的 ST/插件兼容补强包括 common ST message aliases、slash unregister shim、prompt hooks fail-open 和真实复杂卡 smoke 记录。

## 如何验证

Windows 优先跑自动化发布闸门：

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\verify-release.ps1
```

按需复跑专项：

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\run-real-card-smoke.ps1
powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\run-meta-smoke.ps1
powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\run-ui-smoke.ps1
powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\run-android-smoke.ps1
```

真实 LLM 发布验收只从 shell 环境变量读取连接信息，不要写入仓库或文档：

```powershell
$env:LLM_BASE_URL = 'https://your-compatible-endpoint/v1/chat/completions'
$env:LLM_API_KEY = 'use-a-real-key-from-your-shell-only'
$env:LLM_MODEL = 'your-model'
powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\run-real-llm-smoke.ps1 -Suite knowledge
```

手工验收按 `docs/RELEASE-CHECKLIST.md` 执行。每一行都要记录候选版本、日期、平台、输入材料、执行人、结果、失败日志或排障 bundle。没有实跑的项目保持 `待跑` 或 `待真机`，不要提前改成通过。

## 下一优先级

1. 跑 Bronze 桌面主流程矩阵：小卡导入、创建 Campaign、三轮写作、postprocess、Meta explain/patch、重启恢复和排障 bundle。
2. 跑 Silver 真实 ST/MVU 卡矩阵：用 `test-card.png` 和至少一张复杂真实卡补 UI 导入、世界书注入、MVU schema/status bar、regex/HTML 降级和导出记录。
3. 补插件兼容验收：ST 99 事件全集真实触发点、冷门 Slash/TavernHelper 语义、prompt hook 审计 UI/导出、真实插件回归仍未完成。
4. 跑真实 LLM 矩阵：固定模型与参数，记录 T1/T2/T3 质量、耗时、成本、知识隔离/传播对抗和 postprocess 命中情况。
5. 跑 Android 真机矩阵：安装、系统文件选择器导入、主流程、导出 save/share sheet、Android keyring 和长文本/生命周期。
6. 将 `docs/USER-GUIDE.md` 从草案打磨为发布版：补截图或短录屏入口、确认数据目录描述、确认导出入口名称和 Android 差异。

## 交接注意事项

- 本项目当前只需要 Git 提交/推送，不需要回推 VPS 或同步 Termux 文档。
- 不要把真实 API key 写入文档、日志摘要、issue、截图或示例配置；文档中只允许出现 `storyforge-secret:v1:*` 这类 SecretRef 形式。
- 发布说明不要承诺完整 ST 99 事件全集、冷门 Slash/TavernHelper 语义、完整第三方插件沙箱或完整语义级安全边界。
- Prompt hook 可以说“最终 messages 级链路已接入，插件错误/超时 fail-open，基础脱敏审计记录已补”；不要说“完整审计面板、导出和可搜索追踪已完成”。
- 传话链和 private/封口能力当前仍要按“文本匹配级门禁 + 真实 LLM 对抗待验证”描述。
- `CampaignStore` 桌面小/中等数据量暂不阻塞，但 Android、大卡导入和真实长会话必须实测后再决定是否推进后台 flush、分文件索引或 schema 迁移。

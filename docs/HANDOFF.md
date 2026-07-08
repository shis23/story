# StoryForge 交接说明

> 更新日期：2026-07-08
> 范围：发布验收、文档归档与下一步推进。本文只描述当前状态、验证方式和下一优先级，不替代具体实现计划。

## 当前项目状态

- **前端重构 Phase 8 已完成**：App.vue(1462 行单文件)拆分为 60 个 v2 组件 + 4 个 Pinia store + 8 个 composable + 3 纯 util。main.js 已切换到 AppV2 + Pinia。旧 App.vue 及 20 个旧组件已删除;PluginHost/MvuJsRuntime/CharacterList + base/ 因被 v2 或测试引用而保留原位。双轨测试:node --test(212 个,纯 JS)+ vitest(21 个,组件挂载)。契约红线全部保住(141 旧测试护城河 + 16 半脆弱测试 + ChatMessage 8 emit + CampaignPanel refreshActiveDetailTab + MetaPanel mvu-applied)。详见 `docs/FRONTEND-REBUILD-2026-07-08.md`。
- Campaign 主线已经成为写作运行时真相源：Director、Subagent、Editor、Postprocess 都围绕 Campaign/CharacterInstance 工作。
- Agent Profile 与 Character Extraction 的主计划已完成并归档到 `docs/archive/2026-07-08-completed-plans/`；后续增强继续作为验收和质量任务跟踪，不再把这两个 plan 当作当前入口。
- Meta Agent 维护层已完成基础闭环：health check、解释生成、typed patch、preview/accept/dismiss、MVU schema preview/apply 都已接入；MetaPanel 接受 patch 后的状态流已有前端纯模型测试覆盖。
- ST 导入/导出与 MVU 基础能力已落地：V2/V3 导入保真、raw JSON/extensions 保留、Campaign JSON bundle、ST 卡 PNG/共享 lorebook 导出、MVU 状态栏/schema preview、JS fallback runtime 接入写作流程。
- ST/插件兼容已推进到主生成事件、常见聊天事件别名、`eventSource`/`TavernHelper` 常用 shim、Slash 注册/触发/注销、ModifyPrompt 前端 hook、最终 LLM messages 级 prompt hook 和普通事件订阅/正文脱敏。
- Prompt hook 现在有基础审计记录：前端会记录脱敏后的 hook 请求/响应、插件 ID、事件名、耗时、错误摘要和 payload hash/长度；这不是完整的可筛选 UI/导出审计面板。
- UI smoke runner 已加入：`scripts/run-ui-smoke.ps1` 与 `frontend` 的 `npm run smoke:ui` 会在本机已有 `@playwright/test` 时跑浏览器级冒烟；当前环境缺少 Playwright 时会写 `artifacts/ui-smoke/SKIPPED.txt`，不能替代真实 Tauri 桌面 UI 证据。
- 自动化发布基线已建立：`scripts/verify-release.ps1` 覆盖 secret scan、cargo fmt、workspace clippy/tests、frontend test/build；上次完整非沙箱 release gate 已通过，Vite dynamic/static import warning 仍按既有风险记录。
- Android 仍处于打磨阶段：arm64-v8a debug/release 构建链路已有记录，但真机安装、文件导入、share/save sheet、Android keyring 和长会话稳定性仍需现场验收。
- 真实 LLM 矩阵首次实跑通过（2026-07-08，deepseek-v4-flash，endpoint `opencode.ai/zen/go`）：8 个 suite 全绿——knowledge（private 封口 / told_by_other 传话链 / 广播分发 / private 不可二次传播）、i1（子 agent 越权被拦）、t1/t2/t3（首轮 / 多轮 / 三种重 roll）、c1/c6/c7（角色抽取 / meta 对话 / MVU 分析）。实跑同时暴露并修复了两个预存问题：`run-real-llm-smoke.ps1` 在 cargo 输出污染返回值管道 + stderr 触发 Stop 时崩溃（commit `f7f65d6`）；knowledge suite 对广播形态断言过严，LLM 合理输出 `BroadcastTarget::All` 被误判失败（commit `6126a1f`，断言放宽接受 All 或 Group，代码对两者处理均已覆盖）。成本/耗时未做结构化记录，复杂真实卡、长会话稳定性和真实 LLM 对抗仍是发布候选前的补充项。

## 最新提交

真实 LLM 矩阵首次实跑（2026-07-08）:

- `6126a1f test: relax knowledge broadcast assertion for LLM nondeterminism` — knowledge suite 接受 `BroadcastTarget::All` 或 `Group("守卫")`，消除 LLM 非确定性误伤
- `f7f65d6 fix: real-llm smoke runner crashes on cargo output pipeline` — 修 `run-real-llm-smoke.ps1` 在 cargo 输出污染返回值管道 + stderr 触发 Stop 时的崩溃，保证官方真实验证入口可用

前端重构 Phase 8(8 个 commit,详见 `docs/FRONTEND-REBUILD-2026-07-08.md`):

- `ab70021 chore: remove legacy frontend code, finalize phase 8` — 删除旧 App.vue + 20 旧组件,保留 3 个契约文件
- `aca27a3 feat: switch to AppV2 as main entry with pinia` — main.js 切换 AppV2+Pinia
- `b6cda84 feat: rebuild meta/st/config/debug panels (p2/p3)` — meta 6 + config 4 + st 2 + debug 4
- `455be3e feat: rebuild writing workspace + campaign panel + AppV2 assembly` — writing 7 + campaign 7 + AppV2 组装
- `76b7107 feat: extract app.vue logic into composables` — 8 composable + 3 纯 util
- `c6985d1 feat: add ui component library + vitest for component testing` — 24 个 ui 组件 + 双轨测试
- `7e9da68 feat: extract pinia stores from app.vue state` — 4 个 store(campaign/writing/plugin/ui)
- `81e1608 chore: scaffold frontend rebuild v2 (pinia, headless ui, dirs)` — 装依赖 + 建目录

更早的提交(T1-T9 收尾批次,已合入 main):

- `9c9d6de style: cargo fmt`
- `1580fa6 feat: add prompt hook audit export with redaction verification`
- `c54a7cf docs: add st events coverage index`
- `a982614 test: add postprocess writeback boundary regression tests`
- `dcdb04f test: extract campaign tab refresh mapping with test coverage`
- `f3aadf2 test: extract subagent trace pure function with test coverage`
- `c44d69c docs: add regression coverage index with verified test mappings`
- `2369591 docs: standardize release-checklist state markers with unified enum`
- `1e466a6 docs: polish user-guide with screenshot placeholders and code-aligned markers`
- `d4d1ffa test: add ui smoke runner`
- 更早的 ST/插件兼容补强包括 common ST message aliases、slash unregister shim、prompt hooks fail-open 和真实复杂卡 smoke 记录。

## 发布闸门最新记录

2026-07-08 完整六步通过（commit 栈 `ab70021` 前端重构 Phase 8 全部 9 阶段完成）：

1. ✅ secret scan
2. ✅ cargo fmt --check
3. ✅ cargo clippy --workspace --all-targets -- -D warnings
4. ✅ cargo test --workspace（555 pass，真实 LLM 用例按预期 ignore）
5. ✅ frontend npm.cmd test（node --test 212 pass + vitest 21 pass，双轨）
6. ✅ frontend npm.cmd run build（422KB，仅存已知 Vite dynamic import warning）

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
4. 真实 LLM 矩阵首跑已过（2026-07-08，deepseek-v4-flash，8 suite 全绿）。待补强：固定模型与参数的对照记录、T1/T2/T3 的质量/耗时/成本结构化记录、更多真实卡、长会话稳定性和多次对抗取样。脚本与断言脆弱性已随 commit `f7f65d6` / `6126a1f` 修复。
5. 跑 Android 真机矩阵：安装、系统文件选择器导入、主流程、导出 save/share sheet、Android keyring 和长文本/生命周期。
6. 将 `docs/USER-GUIDE.md` 从草案打磨为发布版：补截图或短录屏入口、确认数据目录描述、确认导出入口名称和 Android 差异。

## 交接注意事项

- 本项目当前只需要 Git 提交/推送，不需要回推 VPS 或同步 Termux 文档。
- 不要把真实 API key 写入文档、日志摘要、issue、截图或示例配置；文档中只允许出现 `storyforge-secret:v1:*` 这类 SecretRef 形式。
- 发布说明不要承诺完整 ST 99 事件全集、冷门 Slash/TavernHelper 语义、完整第三方插件沙箱或完整语义级安全边界。
- Prompt hook 可以说“最终 messages 级链路已接入，插件错误/超时 fail-open，基础脱敏审计记录已补”；不要说“完整审计面板、导出和可搜索追踪已完成”。
- 传话链和 private/封口能力当前仍要按“文本匹配级门禁 + 真实 LLM 对抗待验证”描述。
- `CampaignStore` 桌面小/中等数据量暂不阻塞，但 Android、大卡导入和真实长会话必须实测后再决定是否推进后台 flush、分文件索引或 schema 迁移。

# StoryForge 发布检查清单

> 状态：2026-07-07 更新。自动化基线与 workspace clippy 闸门已纳入；Bronze、Silver、真实 LLM、Android 验收改为可执行矩阵。真实卡、真实 LLM、Android 真机和打包结果必须逐项记录，不能用“理论通过”替代。

> 自动化入口：`powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\verify-release.ps1`
> 预检参数：加 `-DryRun` 只打印 release gate 将执行的步骤、工作目录和命令；加 `-SecretScanOnly` 只运行 secret scan。
> 专项入口：真实复杂卡导入 + Campaign bundle roundtrip 冒烟用 `powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\run-real-card-smoke.ps1`；真实 LLM 冒烟用 `powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\run-real-llm-smoke.ps1 -Suite knowledge`；Android host-side 冒烟用 `powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\run-android-smoke.ps1`，打包时追加 `-BuildApk`。

## 0. 发布闸门

任何候选版本必须满足：

自动化发布闸门必须通过 `powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\verify-release.ps1` 执行并通过。脚本按 fail-fast 顺序运行：

1. `secret scan`：扫描 Git-tracked worktree 与 index 文件，只报告规则名和位置，不回显匹配行内容；排除 `target/**`、`node_modules/**`、`frontend/dist/**`、`.git/**`。
2. `cargo fmt --check`。
3. `cargo clippy --workspace --all-targets -- -D warnings`。
4. `cargo test --workspace`。
5. `cd frontend && npm.cmd test`。
6. `cd frontend && npm.cmd run build`。

- `-DryRun` 用于打印上述步骤和命令，不执行 secret scan、Cargo、npm 测试或构建。
- `-SecretScanOnly` 用于快速执行第 1 步；通过后脚本停止，不继续运行格式化、Clippy、测试或构建。
- release notes 明确列出仍需人工验证或降级的能力。
- 用户数据目录、迁移、备份和排障路径已验证。
- 本文件中未完成项不能伪装成已完成；允许标记为“延期/非阻塞”，但必须写明原因。

## 1. 当前自动化基线

2026-07-07 已验证：

- 本轮待推送提交栈已执行 `powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\verify-release.ps1` 并通过完整六步：secret scan、`cargo fmt --check`、workspace clippy、workspace tests、frontend `npm.cmd test`、frontend `npm.cmd run build`。本行只记录自动化基线状态，不作为最终发布 SHA；最终候选 SHA 以 tag/release notes 记录为准。
- 仍有既有 Vite dynamic/static import warning；本轮未新增同类阻塞，继续按分包风险记录，不视为发布闸门失败。
- 真实复杂卡导入 + Campaign bundle roundtrip 专项 smoke 已补：`scripts/run-real-card-smoke.ps1` 默认读取仓库根目录 `test-card.png`，运行 ignored 回归 `test_real_complex_card_fixture_preserves_core_st_fields` 和 `test_real_complex_card_fixture_can_create_campaign_and_roundtrip_bundle`。前者断言卡名、6 个 alternate greetings、441 条世界书、85 个常驻条目、340 个选择性条目，以及 `regex_scripts`、`tavern_helper`、`xiaobaix-template` 等关键 extensions 保留；后者把同一真实卡保存为 `CharacterCard`，创建 Campaign/instances，导出 StoryForge bundle，再导入新 store 并断言关键 extensions、6 个 alternate greetings 和 instances 仍保留。该 smoke 覆盖 S1 的自动导入与 bundle roundtrip 子项，不替代 UI 真实操作和真实写作验收。
- 世界书注入自动语义已补强：`cargo test -p storyforge-domain world_info -- --nocapture` 覆盖 ST `constant=true` + `selective=true` 的 Both 条目导入后同时进入 constant context 和 keyword-triggered selective path；`app-pipeline` / `app-agent` 既有测试覆盖 Director system/tail 注入、未命中不注入和 `search_world_info` 只返回 Selective/Both 匹配。该自动覆盖不替代 S2 的 UI/真实卡端到端验收。
- 专项 smoke runner 已补：`scripts/run-real-llm-smoke.ps1` 统一执行 ignored 真实 LLM 套件并避免打印 API key；`scripts/run-android-smoke.ps1` 统一执行 frontend build、Tauri capability 测试和 Android arm64 host-side check，`-BuildApk` 时再要求 `ANDROID_HOME` / `NDK_HOME`。

2026-07-06 已验证：

- 发布脚本当前覆盖 `secret scan`、`cargo fmt --check`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo test --workspace`、`frontend npm.cmd test`、`frontend npm.cmd run build` 六步；候选版本应以脚本输出为准记录当次结果。
- `secret scan`：覆盖 Git-tracked worktree 与 index 文件；排除 `target/**`、`node_modules/**`、`frontend/dist/**`、`.git/**`；命中时只记录规则名与位置，不写出匹配内容。
- `cargo fmt --check`：作为 Rust 格式闸门。
- `cargo clippy --workspace --all-targets -- -D warnings`：通过，作为 Rust warning-free 闸门。
- `cargo test -p storyforge --lib`：通过。
- `cargo test -p harness-real-llm`：确定性测试通过；真实 LLM 用例默认 ignore。
- `cargo test --workspace`：通过；真实 LLM 用例按预期 ignore。
- `frontend npm.cmd test`：作为前端测试闸门。
- `frontend npm.cmd run build`：通过；若出现 Vite dynamic/static import warning，按现有分包风险记录，不视为本轮阻塞。
- CampaignStore 压测：Git Bash 用 `SF_STORE_PRESSURE_WRITES=500 cargo test -p storyforge --lib pressure_sync_json_io -- --ignored --nocapture`；PowerShell 用 `$env:SF_STORE_PRESSURE_WRITES='500'; cargo test -p storyforge --lib pressure_sync_json_io -- --ignored --nocapture; Remove-Item Env:SF_STORE_PRESSURE_WRITES`。通过；本机 4 集合并发写入 500 次/集合，总耗时约 2.9s，p95 为 knowledge 6.6ms / tasks 7.4ms / summaries 6.8ms / mvu 8.5ms，max 约 28ms。
- API key 安全存储相关测试：`cargo test -p storyforge-infra-util`、`cargo test -p storyforge --lib connection_store`、`cargo test -p storyforge --lib test_embed_config` 通过；覆盖新写入 SecretRef、旧明文迁移、运行时解析和删除清理。Windows Credential Manager 冒烟测试 `cargo test -p storyforge-infra-util system_keyring_write_read_delete_roundtrip -- --ignored --nocapture` 通过；`cargo check -p storyforge-infra-util --target aarch64-linux-android` 通过，Android 后端仍需真机写读删。
- Tauri capability 权限收敛测试：`cargo test -p storyforge --test capabilities` 通过；`default.json` 已移除 `fs:default` / `dialog:default`，仅保留 dialog open/save/message/ask 与 fs read/write file，并静态校验前端文件 helper 与 capability 匹配。Android 真机导入/导出路径仍需手工回归。
- Android 构建链路：主流真机 ABI `aarch64/arm64-v8a` 已通过 `cargo tauri android build --debug --target aarch64 --ci --split-per-abi --apk`（`app-arm64-debug.apk`，约 237 MB）和 `cargo tauri android build --target aarch64 --ci --split-per-abi --apk`（`app-arm64-release-unsigned.apk`，约 39 MB）。此前 x86_64 emulator/universal debug/release 也已通过；armv7/i686 不作为当前发布主线。仍有 Tauri/Gradle/Kotlin deprecation warning、插件 consumer proguard warning 和 macOS `.app` bundle id warning，暂不阻塞本轮 Android 构建基线。
- 排障 bundle 诊断上下文：`cargo test -p storyforge --lib test_diagnostic_context_summarizes_stores_without_secret_values` 通过；`log_export_bundle` 会附带 app/platform、data/log/conversation 路径、关键 store 文件存在性与大小摘要，且不会读取或导出 `connections.json` / `embed.json` 内的 API key。

本轮质量修复：

- `AppState` 支持测试临时数据目录，避免读取本机真实角色和 active Campaign。
- `fill_campaign_context`、`configure_embedder`、`set_active_campaign` 改用 `state.data_dir`。
- `CampaignStore` 写入结果不再静默忽略：Tauri 命令返回 `storage` 错误，postprocess 后台写回记录 warning。
- LLM 连接和 embedder 的 API key 已改为系统凭据库存储；`connections.json` / `embed.json` 只保存 `storyforge-secret:v1:*` 引用，并兼容旧明文文件自动迁移。
- `SystemSecretStore` 显式初始化平台原生凭据库后端，修复真实 Windows keyring 首次使用时报 `No default store has been set` 的问题。
- harness 和 tauri-app 测试 fixture 写入用 `unwrap()` 显式暴露失败。
- Rust workspace/all-targets clippy warning 已清理；保留的高参数公共流程入口只在函数处加局部 allow，避免把 API 重构混入质量闸门切片。
- Tauri Android 动态库补 `#[cfg_attr(mobile, tauri::mobile_entry_point)]`，修复 `failed to validate library` / 缺少 runtime symbols 的构建失败。

## 2. 执行记录规则

每一行验收都需要记录候选版本、执行日期、执行人、平台、输入材料文件名、结果和失败附件位置。状态建议使用：

- `待跑`：发布候选尚未执行。
- `通过`：已按步骤执行并满足预期。
- `失败`：已执行但未满足预期，必须附失败日志或排障 bundle。
- `延期/非阻塞`：本候选版本不阻塞，必须写明原因和降级说明。

失败时优先导出排障 bundle；不要在日志、文档、issue 或截图中写入真实 API key。连接配置文件应只出现 `storyforge-secret:v1:*` 形式的引用。

## 3. Bronze 主流程验收矩阵

桌面端至少跑一张基础卡，证明 Campaign 主链路闭环。建议先用小卡跑通，再用真实卡复跑。

| ID | 输入材料 | 操作步骤 | 预期结果 | 失败日志 / 导出包 | 状态 |
| --- | --- | --- | --- | --- | --- |
| B1 单角色首轮 | 一张 ST V2/V3 单角色 PNG 或 JSON；一个可用 LLM 连接；空白或临时数据目录 | 1. 启动桌面端。<br>2. 导入角色卡。<br>3. 确认角色列表刷新。<br>4. 从该角色创建 Campaign。<br>5. 设为 active Campaign 并写第一轮。 | 导入成功；Campaign 创建成功；Director、Subagent、Editor trace 可见；正文落到当前 Campaign；没有 legacy character 写作路径被误用。 | app 日志；pipeline trace；`log_export_bundle`；导入失败时保留原始卡文件名和错误提示截图。 | 待跑 |
| B2 多角色三轮 | 一张能抽取多个角色定义的 ST 卡或 JSON；同一数据目录保留三轮写作结果 | 1. 导入多角色卡。<br>2. 抽取 definitions。<br>3. 创建含多个 instance 的 Campaign。<br>4. 连续写 T1/T2/T3。<br>5. 每轮后打开 Pipeline 和 Campaign 面板。 | Director 能看到多个 instances；Subagent 分角色输出；Editor 输出正文；三轮后 summaries、knowledge、variables、tasks 至少有一类影响下一轮。 | app 日志；pipeline trace；Campaign bundle；若某轮失败，记录失败轮次和 Agent 阶段。 | 待跑 |
| B3 同名隔离 | 两个同 display name 但不同 instance id 的角色，或能创建同名 instance 的测试 Campaign | 1. 创建两个同名 instance。<br>2. 给其中一个角色制造私有知识或变量。<br>3. 写一轮让 postprocess 写回。<br>4. 查看 knowledge 和 variables tab。 | 知识、变量按 instance id 落盘；同名角色不串写；name 匹配歧义时不会静默写错目标。 | app 日志；Campaign bundle；knowledge/variables 面板截图；相关 pipeline trace。 | 待跑 |
| B4 后处理写回 | B2 的三轮 Campaign，或一张明确会产生知识、变量、任务、摘要的测试卡 | 1. 写作后等待 postprocess 完成。<br>2. 打开 summaries、knowledge、variables、tasks。<br>3. 检查 A→B→C 传话链示例。<br>4. 重启 app 后再次查看。 | 本轮摘要出现；知识面板显示知道者、来源者和 provenance；传话链可显示；变量落在正确 instance 或 campaign；任务创建/完成/放弃可见；重启后 active Campaign 和状态恢复。 | app 日志；`log_export_bundle`；Campaign bundle；重启前后截图。 | 待跑 |
| B5 Meta 解释与修复 | 已有至少一轮生成、trace 和 provenance 的 Campaign；一个可触发 health issue 的小问题 | 1. 打开 Meta 面板。<br>2. 运行 health check。<br>3. 查看“解释本轮生成”。<br>4. 生成 patch preview。<br>5. 分别验证 dismiss 和 accept。 | 解释能引用真实 trace/provenance；patch 先 preview，不直接写核心数据；dismiss 不改状态；accept 后对应 tab 刷新。 | Meta 面板截图；app 日志；patch preview 内容；Campaign bundle。 | 待跑 |
| B6 排障与恢复 | 任一已完成 B1-B5 的 Campaign；一次人为制造的失败场景，如断网或取消生成 | 1. 触发失败。<br>2. 确认 UI 或日志可见错误。<br>3. 导出排障 bundle。<br>4. 重启 app。 | 失败不会伪装成成功；active state 不损坏；导出包包含 app/platform、data/log/conversation 路径和 store 摘要；不泄露真实 API key。 | `log_export_bundle`；app 日志；失败时 UI 文案截图。 | 待跑 |

## 4. Silver ST 兼容验收矩阵

Silver 关注真实 ST/MVU 卡的导入保真、降级可见和状态栏/MVU 基础体验。Regex Slash placement 3 已覆盖 `/` 前缀输入到导演意图的最小 hook；插件桥已提供常用 Slash 注册/触发 fallback、基础 invocation 解析和基础 pipe chaining，`PluginHost` 已提供 per-slot 状态栏/斜杠挂载，并支持声明 `ModifyPrompt` 插件的常驻隐藏 hook host 通过 host→iframe 可等待 prompt hook 改写写作入参。最终 messages 级 prompt hook 已接入写作/重 roll 的 LLM request 前置等待链路；普通事件 feed 已按 `event_subscriptions` 和 `ReadMemory` 做订阅/正文脱敏；完整 ST 冷门 Slash 语义、ST 99 事件全集和 prompt hook 审计日志不作为本轮已完成承诺。

| ID | 输入材料 | 操作步骤 | 预期结果 | 失败日志 / 导出包 | 状态 |
| --- | --- | --- | --- | --- | --- |
| S1 导入保真 | 至少一张复杂 ST/MVU PNG 或 JSON，包含世界书、开场白、标签、extensions 和 raw JSON；自动子项可用仓库根目录 `test-card.png` | 1. 先运行 `powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\run-real-card-smoke.ps1`。<br>2. 在 UI 导入卡。<br>3. 打开角色详情。<br>4. 创建 Campaign。<br>5. 导出 StoryForge Campaign bundle。 | PNG/JSON 导入成功；世界书、开场白、标签、extensions 不丢；`raw_card_json` 保底保留；StoryForge bundle 可导出并可重新导入。 | 导入错误日志；导出的 Campaign bundle；角色详情截图；real-card smoke 输出。 | 自动导入保真 + Campaign/bundle roundtrip 已覆盖，UI 真实操作待跑 |
| S2 世界书注入 | 含 Constant、Selective、Both 世界书条目的卡；两条不同意图的用户输入 | 1. 写一轮命中关键词的输入。<br>2. 写一轮不命中关键词的输入。<br>3. 检查 Director system/tail 摘要。 | Constant/Both 稳定进入 Director system；Selective/Both 按关键词进入 Director tail；未命中条目不注入。 | pipeline trace；Director 输入摘要；app 日志。 | 自动语义已覆盖，UI/真实卡待跑 |
| S3 MVU schema 与状态栏 | 含 MVU 变量定义和状态栏片段的卡；可触发变量变化的一轮写作 | 1. 打开 MVU schema preview。<br>2. 检查新增、覆盖、无变化字段。<br>3. apply schema。<br>4. 写一轮并查看状态栏。 | preview 能区分新增/覆盖/无变化；apply 后变量 tab 刷新；状态栏原生渲染展示关键变量；JS 执行失败时有降级提示。 | Meta/MVU preview 截图；app 日志；Campaign bundle；JS fallback warning。 | schema preview/apply/backfill 与状态栏纯模型自动子项已覆盖；UI 真实操作、真实卡写作与 JS fallback 待跑 |
| S4 Regex/HTML 降级 | 含 `promptOnly`、`markdownOnly`、display-only HTML、`minDepth/maxDepth`、Slash placement 3 和 reasoning 块的卡 | 1. 导入卡并写一轮。<br>2. 用普通输入和 `/` 前缀输入分别触发写作。<br>3. 检查 prompt 注入、消息展示和持久化内容。<br>4. 记录任何降级提示。 | `promptOnly` 不污染显示/存储；`markdownOnly` 不污染 prompt/持久化；display-only HTML 安全渲染；depth 过滤和 `<think>/<thinking>` 处理符合当前实现；Slash placement 3 只作用于 `/` 前缀输入且随后继续执行 Input 正则；不支持路径有清晰提示。 | pipeline trace；消息截图；app 日志；降级记录。 | 待跑 |
| S5 导出兼容 | S1-S4 生成的 Campaign；至少一条知识和一条变量 | 1. 导出 StoryForge JSON bundle。<br>2. 导出 ST 卡 PNG 或共享 lorebook（若入口可用）。<br>3. 重新导入导出物做冒烟检查。 | StoryForge 内部多角色 Campaign 不强行退化成单角色卡；导出物保留必要 Campaign 数据；ST 兼容导出说明降级边界。 | 导出 bundle；重新导入日志；导出文件名和大小记录。 | 待跑 |

## 5. 真实 LLM 验收矩阵

真实 LLM 验收用于发现模型行为波动，不能只看确定性单元测试。每次记录模型、endpoint、温度/采样参数、耗时、重试次数和成本估算。

| ID | 输入材料 | 操作步骤 | 预期结果 | 失败日志 / 导出包 | 状态 |
| --- | --- | --- | --- | --- | --- |
| L1 连接和密钥存储 | 一个真实 LLM 连接；一个 embedder 配置；测试数据目录 | 1. 在 UI 中配置连接和 embedder。<br>2. 写入后重启 app。<br>3. 抽样检查 `connections.json` 和 `embed.json`。<br>4. 删除连接。 | 连接可用；重启后可读；配置文件只保存 `storyforge-secret:v1:*` 引用；删除后凭据清理成功；文档和日志不出现真实 key。 | app 日志；配置文件字段摘要；不要复制真实 key。 | 待跑 |
| L2 复杂卡角色识别 | 至少一张复杂真实卡；真实 LLM 连接 | 1. 导入卡。<br>2. 运行角色识别/抽取。<br>3. 创建 Campaign。 | 产出合理 definitions；抽取失败时 fallback 明确；Campaign 能创建并进入写作。 | app 日志；抽取结果摘要；原始卡文件名；Campaign bundle。 | 待跑 |
| L3 T1/T2/T3 写作质量 | L2 的 Campaign；固定三轮用户输入；评分表 | 1. 写 T1/T2/T3。<br>2. 每轮记录首 token、总耗时、Agent 调用次数。<br>3. 对 Director、Subagent、Editor 输出人工评分。 | Director 选择正确角色；Subagent 只使用可见知识；Editor 保留角色差异；正文连续性可接受；成本和延迟在记录范围内。 | pipeline trace；评分表；app 日志；Campaign bundle。 | 待跑 |
| L4 知识隔离对抗 | 含私有知识和未授权角色的 Campaign；对抗 prompt | 1. 写入或导入私有知识。<br>2. 用对抗 prompt 诱导未授权角色索取秘密。<br>3. 检查正文、trace 和知识写回。 | 未授权角色不能读出私有知识；失败/阻断在日志或 trace 中可见；不把文本匹配级门禁描述成完整语义安全。 | pipeline trace；app 日志；Campaign bundle；对抗 prompt 文本。 | 待跑 |
| L5 知识传播对抗 | harness 真实 LLM 环境；支持 ignored 测试的连接配置 | 1. 在当前 shell 设置 `LLM_BASE_URL`、`LLM_API_KEY`、`LLM_MODEL`。<br>2. 运行 `powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\run-real-llm-smoke.ps1 -Suite knowledge`。<br>3. 保存输出摘要。<br>4. 对失败用例归因。 | 真实 PostProcessor 能抽出定向告知、身份组广播和 private 封口；写回层链路/门禁断言通过；脚本输出不打印 API key。 | 测试输出；harness 日志；失败时相关 fixture 名称。 | 待跑 |
| L6 Postprocess 覆盖 | L3 的三轮 Campaign；至少一轮明确包含事实、变量变化、任务变化和摘要点 | 1. 写作后等待 postprocess。<br>2. 检查 summaries、knowledge、variables、tasks。<br>3. 记录未命中的类别。 | 知识、变量、任务、摘要至少各命中一次；Postprocess 不凭空创建永久事实；失败重试不无限循环。 | app 日志；Campaign bundle；面板截图；成本记录。 | 待跑 |

## 6. Android 验收矩阵

Android 候选版本必须在真机上跑主流程。x86_64 emulator 可保留为历史/辅助基线，armv7/i686 暂不作为当前发布主线。

| ID | 输入材料 | 操作步骤 | 预期结果 | 失败日志 / 导出包 | 状态 |
| --- | --- | --- | --- | --- | --- |
| A1 构建与安装 | arm64-v8a debug APK；arm64-v8a release unsigned APK；一台 Android 真机 | 1. 先运行 `powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\run-android-smoke.ps1`。<br>2. 在已配置 `ANDROID_HOME` / `NDK_HOME` 的机器上运行 `powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\run-android-smoke.ps1 -BuildApk`，或取得候选 APK。<br>3. 安装 debug APK。<br>4. 安装 release unsigned APK 或记录签名阻塞。<br>5. 首次启动。 | host-side smoke 可复现；arm64-v8a debug/release 构建链路可复现；真机可安装或签名阻塞被明确记录；启动不崩溃。 | smoke 输出；Gradle/Tauri 输出；设备型号和 Android 版本；adb logcat 摘要。 | 待真机 |
| A2 文件导入权限 | 一张 ST PNG；一张 ST JSON；Android 系统文件选择器 | 1. 通过系统选择器导入 PNG。<br>2. 通过系统选择器导入 JSON。<br>3. 重启 app 后检查角色仍在。 | capability 收窄后文件读取仍可用；授权路径稳定；导入失败不破坏已有数据。 | adb logcat；app 日志；排障 bundle；失败文件名。 | 待真机 |
| A3 移动端主流程 | A2 导入的角色；真实或测试 LLM 连接；移动网络/Wi-Fi | 1. 创建 Campaign。<br>2. 写第一轮。<br>3. 查看 Pipeline 和 Campaign 面板。<br>4. 写第二轮。 | Android 端能完成导入、创建 Campaign、写作、查看结果；移动布局不依赖桌面宽屏。 | adb logcat；app 日志；pipeline trace；屏幕录制或截图。 | 待真机 |
| A4 导出和分享 | A3 的 Campaign；系统分享/保存入口 | 1. 导出排障 bundle。<br>2. 导出 Campaign bundle。<br>3. 用系统 save/share sheet 保存或分享。 | 导出文件可生成、可保存或分享；排障 bundle 包含诊断上下文摘要；不泄露真实 API key。 | 导出文件名和大小；adb logcat；系统分享失败截图。 | 待真机 |
| A5 Android keyring | Android 真机；一个测试连接配置 | 1. 写入连接。<br>2. 重启 app 后读取。<br>3. 从旧明文配置迁移。<br>4. 删除连接。 | Android keyring backend 可写、可读、可迁移、可删除；配置文件只保留 SecretRef。 | app 日志；配置文件字段摘要；不要导出真实 key。 | 待真机 |
| A6 长文本和生命周期 | 一段长输入；至少三轮写作；后台/前台切换；断网场景 | 1. 写长文本输入。<br>2. 流式输出时切后台再切回。<br>3. 断网或切换网络后重试。 | 长文本流式显示不卡死；WebView 生命周期恢复正常；断网/接口失败文案可理解；不会留下错误 active state。 | adb logcat；app 日志；屏幕录制；排障 bundle。 | 待真机 |
| A7 数据目录与备份 | A3-A6 产生的数据；一次升级或重装前备份动作 | 1. 确认 Android 数据目录策略。<br>2. 导出备份。<br>3. 升级或重装后恢复可见状态。 | 数据目录可写、可迁移、可备份；升级/迁移失败不覆盖旧数据；备份说明可被用户执行。 | 排障 bundle；导出 Campaign bundle；升级前后版本号和日志。 | 待真机 |

## 7. 数据安全验收

- 坏卡导入失败不破坏已有数据。
- 写作中断不会留下错误 active state。
- `CampaignStore` 写入失败能被 UI 或日志观察到。
- `connections.json`、`embed.json` 不应出现真实 API key；发布候选需抽样确认仅包含 SecretRef。
- private/封口知识不会通过 postprocess 的告知或广播写入被继续传播；失败/阻断应在日志中可见。
- 传话链当前依赖文本匹配；发布说明不要把它描述成完整语义级追踪。
- 数据迁移失败不覆盖旧目录。
- 排障 bundle 包含日志和诊断上下文摘要，且不泄露 API key；Android 系统分享/保存链路仍需真机验证。

## 8. 发布遗留风险

当前仍需排队：

- `infra-plugin-host` 的 Tauri 依赖已拆到 `tauri-app/src/mvu_webview_runtime.rs` adapter；发布前继续关注 WebView MVU 真实卡回归。
- `CampaignStore` 已从单 Mutex 拆为集合级锁；桌面压测 500 次/集合通过，暂不因桌面小/中等数据量阻塞发布。Android 设备、真实长会话和大卡导入仍需验证后再决定是否拆后台 flush / `spawn_blocking`。
- API key 明文存储已接入 `keyring`/系统凭据库；Windows Credential Manager 写入/读取/删除已用 ignored 冒烟测试验证。发布前仍需在 macOS/Linux/Android，尤其 Android 真机环境，分别验证凭据写入、读取、迁移和删除。
- 插件事件总线已覆盖主生成链和常见聊天宿主动作，声明 `ModifyPrompt` 的插件可在写作前通过常驻隐藏 hook host 的 `GENERATE_BEFORE_COMBINE_PROMPTS` / `CHAT_COMPLETION_PROMPT_READY` 改写入参，并可在最终 LLM request 前通过后端 `prompt_hook_request` / `plugin_prompt_hook_result` 链路改写 messages；普通事件 feed 已按 `event_subscriptions` 和 `ReadMemory` 控制正文暴露；但还不是 ST 99 事件全集，prompt hook 审计日志与冷门语义仍应避免过度承诺。
- 插件 API 桥已改为调用 `plugin_*` 专用后端命令并注入 `pluginId`，变量读取权限已从写权限中拆出；但变量写入仍保留 `WriteVariables` 直接兼容路径，完整 propose/preview 写入流仍需后续收口，发布说明不要把插件权限描述成完整第三方插件沙箱。
- 秘密/封口机制当前是文本匹配级门禁，不等同完整语义安全边界；发布前仍需真实 LLM 对抗样例确认不会给用户虚假的安全感。
- 真实卡 Gold 档兼容尚未完成验收。

# StoryForge 发布检查清单

> 状态：2026-07-06 初版。自动化基线与 workspace clippy 闸门已纳入；真实卡、真实 LLM、Android 和打包结果仍需逐项记录。

## 0. 发布闸门

任何候选版本必须满足：

- `cargo clippy --workspace --all-targets -- -D warnings` 通过。
- `cargo test --workspace` 通过。
- `cd frontend && npm run build` 通过。
- release notes 明确列出仍需人工验证或降级的能力。
- 用户数据目录、迁移、备份和排障路径已验证。
- 本文件中未完成项不能伪装成已完成；允许标记为“延期/非阻塞”，但必须写明原因。

## 1. 当前自动化基线

2026-07-06 已验证：

- `cargo clippy --workspace --all-targets -- -D warnings`：通过，作为 Rust warning-free 闸门。
- `cargo test -p storyforge --lib`：通过。
- `cargo test -p harness-real-llm`：确定性测试通过；真实 LLM 用例默认 ignore。
- `cargo test --workspace`：通过；真实 LLM 用例按预期 ignore。
- `frontend npm run build`：通过；若出现 Vite dynamic/static import warning，按现有分包风险记录，不视为本轮阻塞。
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

## 2. Bronze 主流程

桌面端至少跑一张基础卡：

- 导入角色卡成功，角色列表刷新。
- 抽取多角色定义成功，能创建 Campaign。
- Campaign 写作 3 轮：
  - Director 能看到 instances。
  - Subagent 只能看到绑定角色信息。
  - Editor 能输出正文。
  - 临场角色成功落盘，下一轮可见。
- 后处理写回：
  - 本轮摘要出现。
  - 知识按角色隔离写入。
  - 知识面板能显示知道者、来源者和 provenance 文案。
  - A→B→C 传话后，知识面板能显示传话链路。
  - 变量更新落在正确 instance 或 campaign。
  - 任务创建/完成/放弃可见。
- 重启 app 后 active Campaign、角色、任务、知识仍可恢复。

## 3. Silver ST 兼容

至少跑一张复杂 ST/MVU 卡：

- PNG/JSON 导入正确，世界书、开场白、标签等字段不丢。
- 世界书 Constant/Both 设定能稳定进入 Director system；Selective/Both 设定能按本轮意图关键词触发进入 Director tail，未命中条目不注入。
- MVU schema preview 正确区分新增、覆盖、无变化字段。
- 状态栏原生渲染可展示关键变量。
- fallback fragments 有清晰提示，不导致写作崩溃。
- regex / HTML 路径的降级结果已记录；regex 的 `promptOnly`/`markdownOnly` 已避免污染错误目标，消息列表 display-only UI 渲染、`minDepth/maxDepth` 过滤和 World Info prompt 作用域已接入，但 PluginHost/HTML 渲染仍待接；legacy 单卡 prompt-template 核心字段宏、本地变量宏、基础动态宏、Campaign 单实例/多实例明确作用域变量宏与 legacy/Campaign alternate greeting 切换已接入；写作/重 roll 的 `PipelineEvent` 已桥到插件事件总线，ST 全量 event_types 与 prompt hooks 仍待补。

## 4. 真实 LLM

使用真实连接跑：

- 角色识别：至少 1 张复杂卡，产出 definitions。
- Campaign 写作 T1/T2/T3：首轮、多轮、重 roll。
- 知识隔离对抗：角色不能读到未授权信息。
- 知识传播对抗：运行 `cargo test -p harness-real-llm knowledge_propagation -- --ignored --nocapture`，确认真实 PostProcessor 能抽出定向告知、身份组广播和 private 封口，并通过写回层链路/门禁断言。
- postprocess：知识、变量、任务、摘要至少各命中一次。

记录模型、endpoint、耗时、失败重试和成本估算。

## 5. Android

Android 候选版本需验证：

- arm64-v8a debug/release 构建链路通过；x86_64 仅作为 emulator/历史基线，armv7/i686 暂不构建以控制磁盘占用。真机安装仍需验证。
- 文件导入权限和路径可用；当前 capability 已收窄，仍需 Android 真机确认系统选择器授权后的 PNG/JSON 读取链路。
- 数据目录可写、可迁移、可备份。
- 长文本流式显示不卡死。
- WebView 生命周期恢复正常。
- 断网/接口失败时错误提示可理解。

## 6. 数据安全

- 坏卡导入失败不破坏已有数据。
- 写作中断不会留下错误 active state。
- `CampaignStore` 写入失败能被 UI 或日志观察到。
- `connections.json`、`embed.json` 不应出现真实 API key；发布候选需抽样确认仅包含 SecretRef。
- private/封口知识不会通过 postprocess 的告知或广播写入被继续传播；失败/阻断应在日志中可见。
- 传话链当前依赖文本匹配；发布说明不要把它描述成完整语义级追踪。
- 数据迁移失败不覆盖旧目录。
- 排障 bundle 包含日志和诊断上下文摘要，且不泄露 API key；Android 系统分享/保存链路仍需真机验证。

## 7. 发布遗留风险

当前仍需排队：

- `infra-plugin-host` 的 Tauri 依赖已拆到 `tauri-app/src/mvu_webview_runtime.rs` adapter；发布前继续关注 WebView MVU 真实卡回归。
- `CampaignStore` 已从单 Mutex 拆为集合级锁；桌面压测 500 次/集合通过，暂不因桌面小/中等数据量阻塞发布。Android 设备、真实长会话和大卡导入仍需验证后再决定是否拆后台 flush / `spawn_blocking`。
- API key 明文存储已接入 `keyring`/系统凭据库；Windows Credential Manager 写入/读取/删除已用 ignored 冒烟测试验证。发布前仍需在 macOS/Linux/Android，尤其 Android 真机环境，分别验证凭据写入、读取、迁移和删除。
- 秘密/封口机制当前是文本匹配级门禁，不等同完整语义安全边界；发布前仍需真实 LLM 对抗样例确认不会给用户虚假的安全感。
- 真实卡 Gold 档兼容尚未完成验收。

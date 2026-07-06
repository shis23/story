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
- `cargo test -p harness-real-llm`：确定性测试通过；真实 LLM 用例因缺少凭证被 ignore。
- `cargo test --workspace`：通过；真实 LLM 用例按预期 ignore。
- `frontend npm run build`：通过；若出现 Vite dynamic/static import warning，按现有分包风险记录，不视为本轮阻塞。

本轮质量修复：

- `AppState` 支持测试临时数据目录，避免读取本机真实角色和 active Campaign。
- `fill_campaign_context`、`configure_embedder`、`set_active_campaign` 改用 `state.data_dir`。
- `CampaignStore` 写入结果不再静默忽略：Tauri 命令返回 `storage` 错误，postprocess 后台写回记录 warning。
- harness 和 tauri-app 测试 fixture 写入用 `unwrap()` 显式暴露失败。
- Rust workspace/all-targets clippy warning 已清理；保留的高参数公共流程入口只在函数处加局部 allow，避免把 API 重构混入质量闸门切片。

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
  - 变量更新落在正确 instance 或 campaign。
  - 任务创建/完成/放弃可见。
- 重启 app 后 active Campaign、角色、任务、知识仍可恢复。

## 3. Silver ST 兼容

至少跑一张复杂 ST/MVU 卡：

- PNG/JSON 导入正确，世界书、开场白、标签等字段不丢。
- MVU schema preview 正确区分新增、覆盖、无变化字段。
- 状态栏原生渲染可展示关键变量。
- fallback fragments 有清晰提示，不导致写作崩溃。
- regex / 宏 / HTML / greeting 路径的降级结果已记录。

## 4. 真实 LLM

使用真实连接跑：

- 角色识别：至少 1 张复杂卡，产出 definitions。
- Campaign 写作 T1/T2/T3：首轮、多轮、重 roll。
- 知识隔离对抗：角色不能读到未授权信息。
- postprocess：知识、变量、任务、摘要至少各命中一次。

记录模型、endpoint、耗时、失败重试和成本估算。

## 5. Android

Android 候选版本需验证：

- debug/release 构建链路通过。
- 文件导入权限和路径可用。
- 数据目录可写、可迁移、可备份。
- 长文本流式显示不卡死。
- WebView 生命周期恢复正常。
- 断网/接口失败时错误提示可理解。

## 6. 数据安全

- 坏卡导入失败不破坏已有数据。
- 写作中断不会留下错误 active state。
- `CampaignStore` 写入失败能被 UI 或日志观察到。
- private/封口知识不会通过 postprocess 的告知或广播写入被继续传播；失败/阻断应在日志中可见。
- 数据迁移失败不覆盖旧目录。
- 排障 bundle 包含足够日志，且不泄露 API key。

## 7. 发布遗留风险

当前仍需排队：

- `infra-plugin-host` 的 Tauri 依赖已拆到 `tauri-app/src/mvu_webview_runtime.rs` adapter；发布前继续关注 WebView MVU 真实卡回归。
- `CampaignStore` 已从单 Mutex 拆为集合级锁；同步 JSON I/O 仍需压测后判断是否拆后台 flush / `spawn_blocking`。
- API key 明文存储仍需 keyring/系统安全存储方案。
- 秘密/封口机制当前是文本匹配级门禁，不等同完整语义安全边界；发布前仍需真实 LLM 对抗样例确认不会给用户虚假的安全感。
- 真实卡 Gold 档兼容尚未完成验收。

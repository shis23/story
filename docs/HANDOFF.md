# StoryForge 交接说明

> 更新日期：2026-07-07
> 范围：发布验收与用户入口文档。本文只描述当前状态、验证方式和下一优先级，不替代具体实现计划。

## 当前项目状态

- Campaign 主线已经成为写作运行时真相源：Director、Subagent、Editor、Postprocess 都围绕 Campaign/CharacterInstance 工作。
- Meta Agent 维护层已完成基础闭环：health check、解释生成、typed patch、preview/accept/dismiss、MVU schema preview/apply 都已接入。
- 前端主工作台已围绕 active Campaign：导入卡、创建 Campaign、写第一轮、查看 Pipeline trace 和 Campaign 状态的主路径已经打通。
- ST 导入/导出与 MVU 基础能力已落地：V2/V3 导入保真、raw JSON/extensions 保留、Campaign JSON bundle、ST 卡 PNG/共享 lorebook 导出、MVU 状态栏/schema preview、JS fallback runtime 接入写作流程。
- 自动化发布基线已建立：workspace clippy、workspace tests、frontend build、CampaignStore 压测、keyring/SecretRef 相关测试、Tauri capability 测试和 Android arm64 构建基线已有记录。
- Android 仍处于打磨阶段：arm64-v8a debug/release 构建已通过，但真机安装、文件导入、share/save sheet、Android keyring 和长会话稳定性仍需现场验收。
- 真实 LLM 仍需发布候选实跑：确定性 harness 已通过，真实 LLM ignore 用例、真实卡、多轮质量、知识隔离/传播对抗和成本记录仍需补齐。
- 发布验收文档已改为矩阵：`docs/RELEASE-CHECKLIST.md` 将 Bronze、Silver、真实 LLM、Android 拆成“输入材料 / 操作步骤 / 预期结果 / 失败日志 / 状态”。

## 如何验证

先跑自动化闸门：

```bash
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cd frontend && npm run build
```

按需复跑专项基线：

```bash
SF_STORE_PRESSURE_WRITES=500 cargo test -p storyforge --lib pressure_sync_json_io -- --ignored --nocapture
cargo test -p storyforge --test capabilities
cargo test -p storyforge --lib test_diagnostic_context_summarizes_stores_without_secret_values
cargo test -p storyforge-infra-util
cargo test -p storyforge --lib connection_store
cargo test -p storyforge --lib test_embed_config
```

真实 LLM 发布验收：

```bash
cargo test -p harness-real-llm knowledge_propagation -- --ignored --nocapture
```

Android 构建基线：

```bash
cargo tauri android build --debug --target aarch64 --ci --split-per-abi --apk
cargo tauri android build --target aarch64 --ci --split-per-abi --apk
```

手工验收按 `docs/RELEASE-CHECKLIST.md` 执行。每一行都要记录候选版本、日期、平台、输入材料、执行人、结果、失败日志或排障 bundle。没有实跑的项目保持 `待跑` 或 `待真机`，不要提前改成通过。

## 下一优先级

1. 跑 Bronze 桌面主流程矩阵：先用小卡验证导入、创建 Campaign、三轮写作、postprocess、Meta explain/patch、重启恢复和排障 bundle。
2. 跑 Silver 真实 ST/MVU 卡矩阵：补至少一张复杂卡的导入保真、世界书注入、MVU schema/status bar、regex/HTML 降级和导出记录。
3. 跑 Android 真机矩阵：优先确认安装、系统文件选择器导入、主流程、导出 save/share sheet、Android keyring 和长文本/生命周期。
4. 跑真实 LLM 矩阵：固定模型与参数，记录 T1/T2/T3 质量、耗时、成本、知识隔离/传播对抗和 postprocess 命中情况。
5. 将 `docs/USER-GUIDE.md` 从草案打磨为发布版：补截图或短录屏入口、确认数据目录描述、确认导出入口名称和 Android 差异。

## 交接注意事项

- 不要把真实 API key 写入文档、日志摘要、issue、截图或示例配置；文档中只允许出现 `storyforge-secret:v1:*` 这类 SecretRef 形式。
- 发布说明不要承诺完整 ST 99 事件全集、prompt hooks、完整 Slash 命令系统/PluginHost JS 斜杠运行时或完整语义级安全边界；当前只覆盖 Regex Slash placement 3 对 `/` 前缀写作意图的最小 hook。
- 传话链和 private/封口能力当前仍要按“文本匹配级门禁 + 真实 LLM 对抗待验证”描述。
- `CampaignStore` 桌面小/中等数据量暂不阻塞，但 Android、大卡导入和真实长会话必须实测后再决定是否推进后台 flush、分文件索引或 schema 迁移。
- 归档文档保持只读；新的发布状态记录写入当前 docs。

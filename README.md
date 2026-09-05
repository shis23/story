# StoryForge

StoryForge 是一个 Android-first 的 AI 多 Agent 协作写作应用。它不是通用聊天壳，而是把角色卡、世界书、角色实例、变量、任务、剧情纪要、长期记忆和多轮分支组织进可持续运行的故事 Campaign。

桌面端 Tauri 主要用于开发、调试、数据迁移和发布验证；移动端仍是长期产品方向。

## 项目定位

- Campaign 是写作运行时的主线真相源。
- Director 规划场景，Subagent 按角色隔离表演，Editor 合并成文。
- Summarizer 生成本轮 Chronicle A；PostProcessor 提取知识、变量和任务更新。
- TurnRecord / TurnAttempt、draft hash、revision CAS 与 Accept 屏障保证正文和 Campaign 状态一致提交。
- Meta Agent 负责解释、诊断、补丁建议和数据健康检查，不参与常规正文生成。
- SillyTavern 角色卡、世界书、插件 API 和 MVU 是兼容输入与扩展层，不是内部领域模型。

## 当前状态

- 当前源码版本为 `0.1.1`。发布范围、当前验收和未闭合项目统一见 [发布状态](docs/RELEASE-STATUS.md)；历史 PASS 不代表后续工作区改动已验收。
- 主界面提供续写（默认）、对手戏和顺序剧组三种生成模式；旧并行 `big_scene` 保留为后端兼容模式，不是每轮都运行完整剧组。
- 多角色 Campaign 写作、重 roll、QualityGate、1× Editor auto-fix、私密知识归属门禁与 Editor redaction 已接入。
- v3 Campaign Bundle 保存正文、变体、终态轮次、源角色卡和本局状态；它是单局故事快照，不是完整应用备份。已采纳历史不可直接改写，当前仅支持已采纳末尾分支。
- Chronicle M0–M4.2.2、ContextEpoch、A/B/C 查询工具和压缩 publication 基础已落地。
- M5 endurance runner 已合入；Gate 6 真实模型证据：Canary3/Coverage12/TextFallback3/Stability30 已 PASS 并 seal；Full100 受 relay 间歇不稳定阻断未完成（r3 跑到 58/100 全健康），Gate 6 已按决议关闭（关闭非 PASS）。
- **默认存储已切换为 SQLite**（Gate 7，2026-08-05）：无配置启动即 SQLite 权威；全新用户初始化空 SQLite 库；旧 JSON 数据启动时自动迁移（缺集合按空导入、孤儿行跳过并计数，迁移前自动备份且不删除旧 JSON）；JSON 保留为显式回退（`STORYFORGE_STORAGE_BACKEND=json`）。
- 已有 Windows 原生、Android 真机基础流程及顶栏专项记录；完整移动端凭据/写作、当前候选安装包和第三方插件验收仍须分别闭合，不能由浏览器测试替代。

## 技术栈

- Rust workspace，16 个 crate。
- Tauri v2，175 个 command。
- Vue 3 + Pinia + Vite + Tailwind v4。
- OpenAI-compatible LLM API。
- 默认 SQLite/WAL 存储；JSON 保留为显式回退与迁移/反向导出源。
- 关键词、向量、ContextEpoch 与 Chronicle A/B/C 记忆路径。

## 下载预编译版本

到 [Releases](https://github.com/shis23/story/releases) 下载最新版：

- **Windows**：`*-setup.exe`（NSIS 安装器）或 `*.msi`，双击安装。首次运行若有 SmartScreen 警告，点「更多信息 → 仍要运行」即可（EXE 未做代码签名，开源项目常见）。
- **Android**：`*-arm64-*-release.apk`，允许「安装未知来源应用」后安装（仅支持 arm64 设备）。

每个版本附 `SHA256SUMS.txt` 校验和。

## 快速启动

环境要求：Windows 11、Node.js v22+（CI 钉 22.12.0；`node --test` glob 语法需 v22 起）、Rust stable（edition 2024）。

```powershell
cd frontend
npm ci
npm run dev
```

另开一个终端运行 `cargo tauri dev`，工作目录为 `crates/tauri-app`。Vite 使用端口 `1420`；纯浏览器预览不提供真实 Tauri 后端能力。构建安装包前先在 `frontend` 执行 `npm run build`。

完整确定性门禁：

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\verify-release.ps1
```

或分别执行：

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cd frontend
npm.cmd test
npm.cmd run test:ui
npm.cmd run test:csp
npm.cmd run test:mobile-chrome
npm.cmd run smoke:ui
npm.cmd run build
```

## 代码结构

```text
crates/domain              领域模型、Turn、Chronicle、NarrativeContract、LLM DTO
crates/app-agent           Agent runtime、工具循环、提示词、质量与后处理解析
crates/app-pipeline        Director → Subagent → Editor 写作编排
crates/app-conversation    对话树、variant、Provenance
crates/app-memory          摘要归档与记忆召回
crates/app-meta            Meta Agent、MVU 分析、补丁会话
crates/infra-*             LLM、SQLite、导入、插件、向量、正则、通用 IO
crates/tauri-app           Tauri 命令、应用服务、JSON/SQLite 组合根
crates/harness-real-llm    确定性与真实模型评估、M5 endurance 证据
frontend                   Vue 3 / Pinia 前端
docs                       当前规格、架构、验收和历史归档
```

## 当前权威文档

- [架构说明](docs/ARCHITECTURE.md)
- [Agent 接口](docs/AGENT_INTERFACES.md)
- [Memory / Context Compiler 规格](docs/MEMORY-CONTEXT-COMPILER-SPEC-2026-07-11.md)
- [Prompt / Cache 架构优化记录](docs/ARCHITECTURE-PROMPT-CACHE-OPTIMIZATION-2026-07-11.md)
- [发布检查清单](docs/RELEASE-CHECKLIST.md)
- [当前发布状态](docs/RELEASE-STATUS.md)

# StoryForge

StoryForge 是一个 Android-first 的 AI 多 Agent 协作写作应用。它的目标不是做一个通用聊天壳，而是把角色卡、世界书、变量、任务、剧情摘要和多轮分支统一进一个可持续运行的故事 Campaign。

桌面端 Tauri 主要用于开发、调试和验证；长期产品形态优先考虑移动端阅读与创作体验。

## 项目定位

StoryForge 的主线是 Campaign：

- SillyTavern 角色卡是导入、兼容和素材来源。
- Campaign 是运行时真相源，承载一局故事的角色实例、变量、知识、任务、摘要和分支。
- Director Agent 规划场景，Subagent 按角色并行表演，Editor Agent 合并成文，Postprocess 将成文写回 Campaign 状态。
- Meta Agent 是诊断、解释和修复层，不是另一个普通聊天入口。

当前代码已经具备多角色 Campaign 模型和后处理雏形，但写作流水线仍主要读取扁平 `Character`。后续开发应先完成 Campaign 主线统一，再扩展外围功能。

## 技术栈

- Rust workspace，14 个 crate。
- Tauri v2，提供本地应用壳、文件导入和 87 个命令。
- Vue 3 + Vite + Tailwind v4 前端。
- OpenAI-compatible LLM API。
- 本地 JSON 存储。
- 内置关键词/向量记忆接口，当前向量实现为 `BruteForceStore`。

## 快速启动

环境要求：

- Windows 11 / Git Bash 或 PowerShell。
- Node.js v24+。
- Rust stable，支持 edition 2024。

安装前端依赖：

```bash
cd frontend
npm install
```

运行前端开发服务：

```bash
npm run dev
```

运行 Tauri 应用：

```bash
cargo tauri dev
```

运行 Rust 测试：

```bash
cargo test --workspace
```

前端构建：

```bash
cd frontend
npm run build
```

## 代码结构

```text
crates/domain              领域模型：角色、Campaign、变量、任务、对话、Agent DTO
crates/app-agent           Agent 运行时、工具循环、提示词、后处理解析
crates/app-pipeline        写作流水线编排：Director -> Subagents -> Editor -> Postprocess
crates/app-conversation    对话树、variant、重 roll Provenance
crates/app-memory          摘要归档与记忆召回
crates/app-meta            Meta Agent、MVU 分析、补丁会话
crates/infra-*             LLM、导入、插件、向量、正则等基础设施
crates/tauri-app           Tauri 命令、本地 store、前后端桥接
frontend                   Vue 前端
docs                       当前架构、数据模型、Agent 契约和路线图
```

## 核心文档

- [架构说明](docs/ARCHITECTURE.md)
- [架构审计与重构建议](docs/ARCHITECTURE-AUDIT.md)
- [文档与代码对齐审计](docs/DOCS-CODE-AUDIT.md)
- [数据模型](docs/DATA_MODEL.md)
- [Agent 接口](docs/AGENT_INTERFACES.md)
- [Campaign 角色统一计划](docs/PLAN-CHARACTER-UNIFICATION.md)
- [Campaign 主线计划](docs/PLAN-CAMPAIGN-MAINLINE.md)
- [Meta Agent 计划](docs/PLAN-META-AGENT.md)
- [前端工作台计划](docs/PLAN-FRONTEND-WORKBENCH.md)
- [Android 计划](docs/PLAN-ANDROID.md)
- [插件与 MVU 计划](docs/PLAN-PLUGIN-MVU.md)
- [主线完成后的收口与发布准备](docs/PLAN-POST-MAINLINE.md)
- [路线图](docs/ROADMAP.md)
- [交接说明](docs/HANDOFF.md)

旧版文档已归档到 [docs/archive/2026-06-16-pre-rewrite](docs/archive/2026-06-16-pre-rewrite)。

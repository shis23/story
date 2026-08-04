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

- 多角色 Campaign 写作、重 roll、QualityGate、1× Editor auto-fix、私密知识归属门禁与 Editor redaction 已接入。
- Chronicle M0–M4.2.2、ContextEpoch、A/B/C 查询工具和压缩 publication 基础已落地。
- M5 endurance runner 已合入；真实 Full 证据为 45/100 Accept，仍是 Partial Evidence。
- 默认存储仍是 JSON；SQLite 是显式 opt-in 后端，已覆盖 cutover、Accept、recovery、barrier 和 reverse export，完整 draft/postprocess 生命周期仍需继续统一。
- 发布脚本、Gitea workflow、导入兼容矩阵和插件兼容矩阵已具备确定性门禁；真实 GUI、Gitea runner、Android 真机和完整生产 Postprocess 证据仍未关闭。

## 技术栈

- Rust workspace，16 个 crate。
- Tauri v2，175 个 command。
- Vue 3 + Pinia + Vite + Tailwind v4。
- OpenAI-compatible LLM API。
- 默认 JSON 存储；opt-in SQLite/WAL 基础设施。
- 关键词、向量、ContextEpoch 与 Chronicle A/B/C 记忆路径。

## 快速启动

环境要求：Windows 11、Node.js v24+、Rust stable（edition 2024）。

```powershell
cd frontend
npm ci
npm run build

cd ..
cargo tauri dev
```

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

- [交接说明](docs/HANDOFF.md)
- [架构说明](docs/ARCHITECTURE.md)
- [Agent 接口](docs/AGENT_INTERFACES.md)
- [Memory / Context Compiler 规格](docs/MEMORY-CONTEXT-COMPILER-SPEC-2026-07-11.md)
- [Prompt / Cache 架构优化记录](docs/ARCHITECTURE-PROMPT-CACHE-OPTIMIZATION-2026-07-11.md)
- [发布检查清单](docs/RELEASE-CHECKLIST.md)
- [M5 100-turn 结果](docs/workstreams/M5-PHASEB-100TURN-EVIDENCE-RESULT.md)

历史计划和旧架构快照位于 [docs/archive](docs/archive/README.md)。

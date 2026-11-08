# StoryForge

**StoryForge** 是一个面向长程叙事的 **AI 多 Agent 协作写作与角色扮演引擎**，同时提供 **DeepSeek Harness (DSH) 插件版** 与 **Windows / Android 独立客户端（Tauri v2）** 两种运行形态。

与传统的单次对话聊天壳不同，StoryForge 以可持续演进的 **故事战役（Campaign）** 为核心状态源，将 **SillyTavern 角色卡、世界书（World Info）、角色实例、动态变量、任务追踪、多层剧情纪要（Chronicle）与长短期记忆** 纳入统一的工程化运行时，通过多角色隔离演绎与后处理记账，解决长篇互动叙事中的“人设崩塌、私密视角串戏、状态遗忘与历史污染”痛点。

---

## 🖥️ 双宿主运行形态（共享 `app-service` 生产内核）

StoryForge 将核心业务与宿主解耦为不依赖特定 UI 框架的 [`crates/app-service`](crates/app-service)，在此之上提供两套完整入口：

| 运行形态 | 核心组件 | 适用场景与特点 |
| :--- | :--- | :--- |
| **🔌 DSH 插件版（重点推荐）** | [`integrations/dsh-storyforge`](integrations/dsh-storyforge) + [`crates/dsh-bridge`](crates/dsh-bridge) | 无缝嵌入 **DeepSeek Harness (DSH)** Web 界面；通过 Rust Sidecar 运行与桌面版完全相同的生产管线与 SQLite 存储；**反向复用 DSH 模型连接（StoryForge 零 API Key 托管）**，支持按管线角色分组指定模型、原位升档确认、卡片界面沙箱与 MVU 状态面板。 |
| **📱 独立应用版（Windows / Android）** | [`crates/tauri-app`](crates/tauri-app) + [`frontend`](frontend) | 基于 **Tauri v2 + Vue 3** 的原生桌面与移动端应用；内置系统级安全凭据库（Windows Credential Manager / Android Keystore）、Card Studio 工作台与完整独立 UI。 |

---

## ✨ 核心特性

- 🎬 **多 Agent 剧组式写作管线**
  - **三种生成档位**：支持 **续写**（单次快速成文，默认）、**对手戏**（双人焦点互动）与 **顺序剧组**（多角色出场大场面），并提供自动路由与高成本群像档位确认。
  - **职责解耦**：由 **Director（导演）** 规划分镜与出场调度 → **Character Subagent（角色子代理）** 在视角隔离的上下文中独立演绎言行与心理 → **Editor（执笔/编剧）** 汇总成文 → **QualityGate（质量门禁）** 自动校验视角越界、格式泄漏并支持 1× 自动修订（Auto-fix）。
- 🧠 **分层记忆与私密知识归属（Provenance）**
  - **Chronicle A / B / C 三层纪要**：每轮自动生成本轮明细摘要（Chronicle A），随轮次推进自动压缩为阶段纪要（B）与全局主线纪要（C）。
  - **Context Compiler & 混合召回**：结合关键词、向量语义检索与 `ContextEpoch` 缓存对齐，按角色认知边界动态编译上下文，严防角色“全知全能”或偷看他人的私密设定。
- 🔒 **事务级轮次屏障与分支演进**
  - **草稿与定稿隔离**：通过 `TurnRecord / TurnAttempt`、revision CAS 与采纳屏障保证正文与 Campaign 状态一致提交；**Summarizer（摘要器）** 与 **PostProcessor（后处理器）** 自动提取新知识、变量变更与任务进度并原子化落盘至 SQLite（WAL）。
  - **不可变历史与末尾分叉**：已采纳的正文受前缀保护不可直接篡改，支持从已采纳末尾轮次创建平行故事分支（Fork），或导出包含源角色卡与完整状态的 `v3 Campaign Bundle` 快照。
- 🧩 **SillyTavern 生态兼容与卡片安全沙箱**
  - 原生支持导入/导出 SillyTavern `.png` / `.json` 角色卡与世界书，支持卡片正则（Regex）展示转换。
  - 支持 **MVU（MagVarUpdate）状态栏原生解析渲染**、卡片内联/远程界面（`sandbox="allow-scripts"` 不透明源隔离 iframe，涵盖自定义开局界面选开场、世界书条目开关、输入框草稿填充与界面状态持久化）。
- 🩺 **Meta Agent 诊断与调试工作台**
  - 内置独立于正文生成的 **Meta Agent**，提供剧情状态解释、设定冲突诊断、状态补丁建议（Patch Session）与全链路 LLM 调用追踪。

---

## 🔌 DeepSeek Harness (DSH) 插件版详解

[`integrations/dsh-storyforge`](integrations/dsh-storyforge) 将完整的 StoryForge 多 Agent 写作引擎作为插件接入 DeepSeek Harness（DSH）。详情与完整操作手册请参阅 👉 [**DSH 插件版专属文档 (`integrations/dsh-storyforge/README.md`)**](integrations/dsh-storyforge/README.md)。

### 1. 反向 LLM 桥接架构（零 API Key 托管）

```mermaid
sequenceDiagram
    participant UI as DSH Web 界面 (client.js)
    participant Host as DSH 插件宿主 (index.js / bridge.js)
    participant Sidecar as Rust Sidecar (storyforge-dsh-bridge)
    participant LLM as DSH 模型服务商 (ctx.llm.stream)

    UI->>Host: 发送玩家行动 / 选择生成模式
    Host->>Sidecar: stdio JSON-RPC (turn.run / route_generation)
    opt 自动路由建议升至高成本群像档
        Sidecar-->>Host: confirm_generation_mode (预计调用量与理由)
        Host-->>UI: 经 DSH userQuestions 在输入框原位确认
        UI->>Host: 确认档位 (群像 / 续写 / 取消)
    end
    loop 导演 / 角色子代理 / 执笔 / 质量门 / 摘要 / 后处理
        Sidecar->>Host: 反向 JSON-RPC 请求 (llm.stream + 系统提示词)
        Host->>LLM: 按管线角色分组路由模型并流式调用
        LLM-->>Sidecar: 流式返回 Token / 原生 Tool Calls
        Sidecar-->>UI: 实时推送思考进度 (导演/角色/执笔) 与正文
    end
    Sidecar->>Sidecar: 后处理记账 & 自动 Accept 落盘 SQLite
    Sidecar-->>UI: 更新右栏 MVU 状态面板 & 挂载回合尾部卡片界面
```

### 2. DSH 插件版核心亮点

- **零密钥配置，按角色分组指定模型**：StoryForge Sidecar 自身不保存任何 API Key，所有模型请求均反向交由 DSH 执行。支持在插件首页为 **导演、角色表演、成文（执笔/编剧）、后台收尾（摘要/后处理/纪要压缩）、导入与分析（角色识别/MVU 分析）** 五大职责组分别指定不同底层模型（例如导演与收尾用高速轻量模型、成文用旗舰模型），并支持分组连接测试与调用明细追踪。
- **输入框原位升档确认**：当生成模式设为「自动路由」且引擎判断本轮需升级为高成本顺序群像时，插件直接通过 DSH 的 `userQuestions` 通道在会话输入框处弹出确认（0 次模型预消耗），一键选择「用群像 / 按续写 / 取消本轮」。
- **完整的卡片界面（卡壳）与 MVU 原生面板**：
  - **回合尾部渲染**：每轮回复下方自动渲染经 DOMPurify 净化并隔离在 Shadow DOM 内的状态栏 HTML，以及运行在 `sandbox="allow-scripts"` 不透明源 iframe 中的复杂卡片界面（与桌面端共用同一套 `card-shell` 信任判定、CSP 响应头与白名单代取缓存）。
  - **开场界面与世界书联动**：支持复杂角色卡（如带 DLC 开关与多路线选择的自定义开局卡）在开局前挂载首页界面，通过酒馆助手兼容接口读写本局世界书条目启用状态、切换开场白，或通过 `createChatMessages` 将生成的角色设定填入 DSH 输入框草稿。
  - **右栏「卡内状态」面板**：支持一键分析卡内 MVU 变量脚本、应用变量定义，实时展示进度条/标签/文字状态行，并通过交互按钮修改角色/全局变量或发送下一轮提示。
- **DSH 会话生命周期无缝映射**：
  - 在 DSH 中从最后一条回复点击 **「分叉」**，子会话首次发送时自动映射为 StoryForge 存档的平行分支（Campaign Fork）并同步分支标题。
  - 在 DSH 侧栏归档/取消归档会话，插件首页自动同步至「已归档的故事」分组；Sidecar 进程若意外中断，下一次请求自动拉起并恢复中断轮次，无需重启 DSH。

### 3. 在 DSH 中快速体验插件

```bash
# 1. 构建 Rust Sidecar (storyforge-dsh-bridge)
cargo build -p storyforge-dsh-bridge

# 2. 在隔离的 DSH_HOME 中初始化 web profile 并挂载插件
cd /path/to/dsh-poc/workspace && DSH_HOME="/path/to/dsh-poc" dsh --profile sfpoc --from-default-profile web --dump-config > /dev/null
DSH_HOME="/path/to/dsh-poc" dsh plugin --profile sfpoc add "link:/path/to/storyforge/integrations/dsh-storyforge"

# 3. 启动 DSH 实例
cd /path/to/dsh-poc/workspace && DSH_HOME="/path/to/dsh-poc" dsh --profile sfpoc --port 0 --no-open
```

也可以将 Sidecar 与插件本体打包为独立的平台分发包（无需仓库源码环境即可安装）：

```bash
node integrations/dsh-storyforge/scripts/pack-bridge.mjs
# 产物输出至 target/dsh-dist/（含当前平台二进制包 dsh-plugin-storyforge-bridge-<platform>-<arch> 与插件本体 tarball）
```

---

## 📦 独立应用版下载与安装（Windows / Android）

前往 [**GitHub Releases**](https://github.com/shis23/story/releases) 下载最新预编译安装包（当前发布版本：`v0.1.2`）：

| 平台 | 安装包类型 | 安装说明 |
| :--- | :--- | :--- |
| **Windows (x64)** | `StoryForge_*_x64-setup.exe` / `*.msi` | 双击安装即可。由于开源构建暂未购买商业代码签名证书，首次运行若弹出 Windows SmartScreen 提示，点击 **「更多信息 → 仍要运行」** 即可。 |
| **Android (arm64)** | `app-arm64-release.apk` | 适用于 64 位 ARM 安卓设备，允许「安装未知来源应用」后直接安装。 |

> **校验和说明**：Release 页面提供构建产物的 SHA-256 校验文件（`SHA256SUMS*.txt`），下载后可核对文件哈希以确保完整性。

---

## 🚀 本地开发与测试

### 环境要求
- **操作系统**：Windows 11（桌面端开发与门禁脚本运行环境）
- **Node.js**：`v22+`（CI 锁定 `22.12.0`）
- **Rust**：`stable`（Rust Edition 2024）

### 启动独立桌面端开发环境

1. 安装前端依赖并启动 Vite 开发服务器（固定端口 `1420`）：
   ```powershell
   cd frontend
   npm ci
   npm run dev
   ```
2. 另开一个终端启动 Tauri 桌面端宿主：
   ```powershell
   cd crates/tauri-app
   cargo tauri dev
   ```
   *(或者在 Git Bash 下直接运行根目录的 `./dev.sh` 一键拉起前后端。)*

### 运行测试与质量门禁

```powershell
# DSH 插件版与 Sidecar 专项测试（Rust 端到端测试 + Node 宿主/前端 71 项测试）
cargo test -p storyforge-dsh-bridge
node --test integrations/dsh-storyforge/test/*.test.mjs

# 运行完整 11 步确定性发布门禁（需安装 Pester、PyYAML==6.0.2 及 Playwright Chromium）
powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\verify-release.ps1
```

也可以按模块单独执行常用检查：

```powershell
# Rust 格式化、静态检查与全工作区测试
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace

# 前端单元测试、UI 组件测试与生产构建
cd frontend
npm test
npm run test:ui
npm run build
```

---

## 📂 代码库结构

本仓库采用 **Rust Workspace（18 个 Crate）+ DSH 插件 + Tauri v2 + Vue 3 (Pinia / Vite / Tailwind v4)** 架构：

```text
integrations/
└── dsh-storyforge/        # DSH 插件版（宿主适配器、Web 界面、卡片沙箱壳服务端、MVU 面板与打包脚本）

crates/
├── dsh-bridge             # DSH 插件版的 Rust stdio JSON-RPC Sidecar（复用 app-service 与 SQLite）
├── app-service            # 宿主无关的应用服务层与 JSON/SQLite 组合根（DSH 与 Tauri 共用内核）
├── domain                 # 核心领域模型、Turn 状态机、Chronicle、NarrativeContract、LLM DTO
├── app-agent              # Agent 运行时、工具调用循环、提示词组装、质量门与后处理解析
├── app-pipeline           # Director → Subagent → Editor 多档位写作编排管线
├── app-conversation       # 对话树、候选变体（Variant）、知识归属（Provenance）
├── app-memory             # 摘要归档、向量/关键词混合检索与 Context Compiler
├── app-meta               # Meta Agent 诊断、MVU 规则分析、状态补丁会话
├── app-logging            # 结构化遥测、LLM 调用详情记录与脱敏导出
├── infra-llm              # OpenAI-compatible 流式/非流式客户端、重试与工具协议降级
├── infra-sqlite           # 默认 SQLite (WAL) 存储实现、Schema 迁移、备份与只读快照导出
├── infra-st-import        # SillyTavern 角色卡（PNG/JSON）与世界书解析导入
├── infra-plugin           # 第三方插件清单校验、权限门控与审计日志
├── infra-vector           # 嵌入向量索引与相似度检索
├── infra-regex            # 安全的正则替换引擎
├── infra-common           # 原子文件 IO、凭证引用（SecretRef）与通用基础设施
├── tauri-app              # Tauri v2 桌面/移动端外壳（179 个 Command）、系统安全凭据库、卡片壳协议
└── harness-real-llm       # 确定性仿真与真实模型端到端评估套件

frontend/                  # Vue 3 + Pinia + Tailwind v4 独立前端应用（与 DSH 插件共用卡片壳模块）
docs/                      # 架构设计、领域规格、用户指南与发布验收文档
scripts/                   # 发布门禁、安全扫描与自动化验证脚本
```

---

## 📚 文档导航

| 文档 | 内容说明 |
| :--- | :--- |
| [**DSH 插件版说明 (`integrations/dsh-storyforge/README.md`)**](integrations/dsh-storyforge/README.md) | **DSH 插件版架构、模块职责、隔离运行指南、本地打包与已知限制说明** |
| [**用户使用指南 (`docs/USER-GUIDE.md`)**](docs/USER-GUIDE.md) | 首次配置模型连接、导入角色卡、三种写作模式使用、分支与快照备份说明 |
| [**系统架构说明 (`docs/ARCHITECTURE.md`)**](docs/ARCHITECTURE.md) | 分层架构、模块边界、核心数据流与设计原则 |
| [**技术设计文档 (`docs/TECHNICAL_DESIGN.md`)**](docs/TECHNICAL_DESIGN.md) | 写作管线、状态一致性、错误处理与关键技术决策 |
| [**数据模型规格 (`docs/DATA_MODEL.md`)**](docs/DATA_MODEL.md) | Campaign、Character、Turn、Chronicle 及 SQLite 存储结构定义 |
| [**Agent 接口契约 (`docs/AGENT_INTERFACES.md`)**](docs/AGENT_INTERFACES.md) | Director、Subagent、Editor、Summarizer、PostProcessor 与 Meta Agent 契约 |
| [**记忆与上下文编译器 (`docs/MEMORY-CONTEXT-COMPILER-SPEC-2026-07-11.md`)**](docs/MEMORY-CONTEXT-COMPILER-SPEC-2026-07-11.md) | Chronicle A/B/C 压缩层级与上下文动态组装规格 |
| [**Prompt 与缓存优化 (`docs/ARCHITECTURE-PROMPT-CACHE-OPTIMIZATION-2026-07-11.md`)**](docs/ARCHITECTURE-PROMPT-CACHE-OPTIMIZATION-2026-07-11.md) | `ContextEpoch` 稳定性与前缀缓存命中率优化设计 |
| [**前端组件架构 (`docs/FRONTEND-COMPONENTS.md`)**](docs/FRONTEND-COMPONENTS.md) | Vue 3 前端视图、Store 状态管理与 Tauri IPC 接线说明 |
| [**变量与状态系统 (`docs/CAMPAIGN-VARIABLES.md`)**](docs/CAMPAIGN-VARIABLES.md) | Campaign 变量、MVU 状态栏解析与卡片壳交互机制 |
| [**产品路线图 (`docs/ROADMAP.md`)**](docs/ROADMAP.md) | 阶段演进记录与后续规划 |
| [**当前发布状态 (`docs/RELEASE-STATUS.md`)**](docs/RELEASE-STATUS.md) | 当前版本（`0.1.2`）验收范围、DSH 插件合入记录与已知限制边界 |
| [**发布检查清单 (`docs/RELEASE-CHECKLIST.md`)**](docs/RELEASE-CHECKLIST.md) | 发版前质量门禁与真机验证核对清单 |

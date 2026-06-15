# StoryForge

> AI 多 Agent 协作写作 App —— 抛弃单 prompt 注入，改用多 Agent 显式编排联合写文。

**平台**：Android（主力） / 桌面端（开发调试）  
**状态**：M1 核心功能完成，M2 记忆系统接入中，Android 构建链路已打通

---

## 核心理念

传统 SillyTavern 式写作靠「预设 + 提示词注入 → 单模型生成」，所有信息挤进一个 prompt。

StoryForge 改用**多 Agent 编排**：

```
用户写作意图
    ↓
导演 Agent（强模型）→ 检索世界书/记忆 → 拆解任务 → 输出 Plan
    ↓ 并行派发
┌───┼───┐
子A  子B  子C（廉价模型，每角色一个）→ 各自表演
└───┬───┘
    ↓
编剧 Agent（强模型）→ 收集子产出 → 合并润色 → 成文
```

**独有能力**：
- **部分重 roll**：只重跑某个子 Agent，省 60% token
- **专属上下文包**：导演为每个子 Agent 构造不污染的上下文
- **世界书路由可调**：每条目可独立设置常驻/向量池/两者/禁用
- **三层记忆系统**：近期原文 → LLM 压缩摘要 → 向量检索池

---

## 技术栈

| 层 | 技术 |
|----|------|
| 后端 | Rust + Tokio |
| 前端 | Vue 3 + Vite + Tailwind v4 |
| 框架 | Tauri v2（移动端 + 桌面端） |
| LLM 协议 | OpenAI 兼容（DeepSeek / SiliconFlow / OpenAI / 自定义） |
| 嵌入模型 | Qwen3-Embedding-8B（SiliconFlow，4096 维） |
| 向量存储 | BruteForceStore（余弦相似度，JSON 持久化） |
| 包管理 | Cargo（Rust）+ npm（前端） |

---

## 项目结构

```
storyforge/
├── Cargo.toml                          # workspace（14 个 crate）
├── crates/
│   ├── domain/                         # 纯领域模型（无 IO）
│   │   ├── character.rs                # 角色卡（ST V2/V3 兼容）
│   │   ├── world_info.rs              # 世界书（蓝灯/绿灯/路由）
│   │   ├── preset.rs                  # 预设（提示词+正则脚本）
│   │   ├── llm.rs                     # LLM 连接/消息/工具/错误
│   │   ├── agent.rs                   # Agent 角色/Plan/流水线状态
│   │   ├── prompt_module.rs           # 提示词模块/Profile/绑定
│   │   └── conversation.rs            # 对话树（MessageNode/Variant）
│   ├── infra-import/                   # ST 数据导入（PNG/JSON）
│   ├── infra-llm/                      # LLM 客户端
│   │   ├── http_client.rs             # HttpLlmClient（reqwest + SSE）
│   │   ├── mock_client.rs             # MockLlmClient（测试用）
│   │   ├── sse.rs                     # 自研 SSE 解析器
│   │   ├── openai.rs                  # OpenAI 兼容协议
│   │   ├── text_tools.rs              # XML/JSON 降级工具协议
│   │   └── embedder.rs               # 嵌入 API 客户端
│   ├── infra-vector/                   # 向量存储（BruteForceStore）
│   ├── infra-regex/                    # 正则引擎（regress）
│   ├── infra-plugin-host/             # 插件运行时骨架
│   ├── app-logging/                    # 日志系统（三类：后端/LLM/前端）
│   ├── app-agent/                      # Agent 运行时（工具循环+委派+取消）
│   ├── app-conversation/               # 对话管理（持久化+编辑/删除/重roll）
│   ├── app-pipeline/                   # 写作流水线编排（状态机）
│   ├── app-memory/                     # 记忆系统（归档器+召回器）
│   ├── app-meta/                       # Meta Agent（诊断+Patch）
│   └── tauri-app/                      # Tauri 入口（32 个命令）
└── frontend/                           # Vue 3 前端
    └── src/
        ├── App.vue                     # 主应用
        ├── tauri-api.js                # Tauri IPC 桥（32 个命令）
        └── components/
            ├── ChatMessage.vue         # 对话消息（编辑/采纳/删除/分支/重roll）
            ├── PipelinePanel.vue       # 流水线状态面板
            ├── CharacterDetail.vue     # 角色详情（含世界书路由编辑）
            ├── ConnectionConfig.vue    # LLM 连接配置
            ├── LogPanel.vue            # 日志面板
            └── ...
```

---

## 快速开始

### 环境要求

- Rust 1.95+（stable）
- Node.js 24+
- JDK 17（Android 构建需要）
- Android SDK + NDK 27.2（Android 构建需要，已配置完成）

### 桌面端开发

```bash
cd storyforge

# 启动前端 dev server
cd frontend && npm run dev

# 另一个终端：启动 Tauri 桌面端
cd crates/tauri-app && cargo tauri dev
```

### 运行测试

```bash
cd storyforge

# 全 workspace 测试（87 个）
cargo test --workspace

# 单 crate 测试
cargo test -p storyforge-infra-llm
cargo test -p storyforge-app-conversation
cargo test -p storyforge-app-pipeline
```

### 构建前端

```bash
cd storyforge/frontend
npm run build
```

### Android 构建（已配置完成）

```bash
cd storyforge/crates/tauri-app

# 环境变量
export ANDROID_HOME="C:/Users/Predator/android-sdk"
export JAVA_HOME="C:/Program Files/Eclipse Adoptium/jdk-17.0.19.10-hotspot"

# Rust 交叉编译
cargo tauri android build --target x86_64

# 复制 .so（Windows 符号链接问题）
cp target/x86_64-linux-android/release/libstoryforge_lib.so \
   crates/tauri-app/gen/android/app/src/main/jniLibs/x86_64/

# Gradle 打包 APK
cd crates/tauri-app/gen/android
./gradlew assembleX86_64Debug -x rustBuildX86_64Debug

# APK 位置
ls app/build/outputs/apk/x86_64/debug/app-x86_64-debug.apk
```

---

## 42 个 Tauri 命令

<details>
<summary>点击展开完整列表</summary>

**角色卡管理**
- `import_character` — 导入角色卡（PNG/JSON，自动检测格式）
- `list_characters` — 列出已导入的角色卡
- `get_character` — 获取角色卡详情（含世界书条目）
- `delete_character` — 删除角色卡
- `update_world_info_route` — 更新世界书条目路由（蓝灯/绿灯/Both/禁用）
- `import_preset` — 导入预设
- `get_version` — 获取版本号

**LLM 连接管理**
- `list_connection_templates` — 列出内置连接模板
- `list_connections` — 列出已配置的连接
- `get_active_connection` — 查询当前活跃连接
- `create_connection` — 创建连接
- `delete_connection` — 删除连接
- `set_active_connection` — 设为活跃
- `test_connection` — 测试连通性

**写作流水线**
- `start_writing` — 启动写作流水线
- `cancel_writing` — 取消当前写作

**对话操作**
- `list_conversations` — 列出所有对话
- `get_conversation` — 获取对话详情
- `regenerate` — 重 roll（整体/只重编剧/只重子Agent，可附 hint）
- `edit_variant` — 编辑当前变体内容
- `accept_variant` — 采纳变体（Draft → Final）
- `soft_delete_variant` — 软删除变体
- `add_variant` — 添加新变体（分支）
- `switch_variant` — 切换变体

**日志系统**
- `log_query` — 查询日志
- `log_clear` — 清空日志
- `log_export_bundle` — 导出脱敏日志 bundle
- `log_append_frontend` — 前端日志上报

**记忆系统（M2）**
- `configure_embedder` — 配置嵌入 API
- `get_embed_config` — 获取嵌入配置
- `archive_conversation` — 手动触发对话归档

**Meta Agent（M5）**
- `meta_accept_patch` — 执行 Meta Agent Patch

**后处理流水线（P2）**
- `list_character_knowledge` — 列角色可见信息（character_knowledge，按 campaign/角色筛选）
- `list_tasks` — 列叙事计划任务（按状态筛：pending/active/completed/abandoned）
- `create_task` — 用户手动建任务（伏笔/目标，含触发条件）
- `complete_task` — 标记任务完成（覆盖 Agent 判断）
- `abandon_task` — 放弃任务
- `list_round_summaries` — 列本轮剧情摘要（按 turn 升序，200-500 字/条）

</details>

---

## 架构设计

### 多 Agent 流水线

```
Idle → Directing → Delegating → Editing → Review → Committed
         ↑              ↑            ↑
       导演Agent     子Agent×N    编剧Agent
       (强模型)     (廉价模型)    (强模型)
```

每个阶段可独立取消，子 Agent 并发上限 4 个。

### 对话树（非线性）

```
ST: Message { swipes: ["v1","v2","v3"], swipe_id: 1 }  ← 线性
StoryForge: MessageNode { variants: [v1,v2,v3], active_variant: 1 }  ← 树形
```

每个 variant 带 Provenance（溯源信息），支持部分重 roll。

### 三层记忆系统

```
Layer 1: Recent Window（近期原文）→ 滑动窗口
    ↓ 窗口溢出触发归档
Layer 2: Archived Summary（远记忆摘要）→ LLM 压缩（≤500TK）
    ↓ 每条摘要向量化
Layer 3: Vector Index（向量检索池）→ 余弦相似度搜索
```

### 世界书路由

每条世界书条目可独立设置路由：
- 🔵 **蓝灯（Constant）**：进导演常驻上下文
- 🟢 **绿灯（Selective）**：进向量检索池
- 🔵🟢 **两者（Both）**：常驻 + 向量检索
- ⚫ **禁用（Disabled）**：不使用

### 数据兼容

| SillyTavern 数据 | StoryForge 支持 |
|------------------|----------------|
| 角色卡（PNG/JSON，V2/V3） | ✅ 完整导入 |
| 世界书（蓝灯/绿灯） | ✅ 完整导入 + 路由可调 |
| 预设（提示词+正则） | ✅ 完整导入 |
| ST 插件 | ❌ 不做（DOM 依赖） |
| 酒馆助手脚本 | ❌ 不做（自有插件运行时） |

---

## 配置文件

| 文件 | 位置 | 说明 |
|------|------|------|
| `characters.json` | `data/` | 已导入的角色卡 |
| `connections.json` | `data/` | LLM 连接配置（含 API key，桌面开发阶段明文） |
| `conversations/` | `data/` | 对话持久化（每对话一个 JSON） |
| `vectors.json` | `data/` | 向量存储持久化 |
| `embed.json` | `data/` | 嵌入 API 配置 |
| `logs/` | `data/` | 落盘日志（ERROR + LLM 调用） |

---

## 设计文档

| 文档 | 说明 |
|------|------|
| [INTENT.md](docs/INTENT.md) | 需求决策文档（48 条决策，D1-D48） |
| [TECHNICAL_DESIGN.md](docs/TECHNICAL_DESIGN.md) | 技术方案设计（23 章，含角色隔离/Campaign/MVU/叙事计划/cache 布局/变量体系） |
| [HANDOFF.md](docs/HANDOFF.md) | 项目交接文档（当前状态 + 后续计划 + 决策时点） |
| [AGENT_INTERFACES.md](docs/AGENT_INTERFACES.md) | Agent 接口索引（所有 prompt/上下文/输出解析位置，改 prompt 只看这文件） |

---

## 参考项目

- [SillyTavern](https://github.com/SillyTavern/SillyTavern) — 上游原版，Web 应用
- [TauriTavern](https://github.com/Darkatse/TauriTavern) — ST 的 Tauri 移植
- [LittleWhiteBox](https://github.com/RT15548/LittleWhiteBox) — 小白X 插件
- [JS-Slash-Runner](https://github.com/n0vi028/JS-Slash-Runner) — 酒馆助手

---

## 许可证

私有项目，未开源。

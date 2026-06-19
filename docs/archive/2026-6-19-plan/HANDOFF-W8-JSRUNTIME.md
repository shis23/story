# W8 执行手册：JS Fallback WebView Runtime + js-slash-runner 常用子集 shim

> 交接对象：Claude Code（在 worktree `storyforge-w8-jsruntime` 分支 `w8-jsruntime` 工作）
> 前置必读：`docs/PLAN-PLUGIN-MVU.md` 阶段 5、`crates/infra-plugin-host/src/mvu_runtime.rs`
> 工作目录：`C:\Users\Predator\ZCodeProject\storyforge-w8-jsruntime`
> 分支：`w8-jsruntime`（已基于 `main` 1945d21）
> 性质：后端 runtime + 前端 WebView 容器，与 W7（导出）零文件交集。

## 一、任务概述

MVU 卡里 Meta Agent 翻译不了的 JS 片段（`fallback_fragments` 非空 + `routing = Hybrid`）需要执行环境跑原 JS。现状 `StubMvuRuntime` 全返回 NotImplemented。

用户要求：**兼容 js-slash-runner（JSR）的脚本**。已定范围：**JSR/ST API 常用子集**（非完整兼容——完整是永远追不完的黑洞）。

## 二、技术路径（已定，不要偏离）

**必须用 WebView runtime，不要用 QuickJS。** 原因：JSR 脚本依赖浏览器 DOM（document/jQuery）+ ST 专属 API + JSR 自有 API。QuickJS 无 DOM，跑不了 JSR 脚本。`mvu_runtime.rs:8` 注释已写明 QuickJS 只能跑"零星无 DOM 片段"，重 DOM 卡（如缄默之秋）必须共享 WebView。

实现路径：**Tauri WebView 隐藏容器 + JSR/ST API shim 注入**。
- 前端开一个隐藏的 WebView（iframe 或隐藏 div + 脚本注入），加载卡的 HTML+JS。
- 注入一个 shim 层，模拟 JSR/ST 常用 API（见下）。
- 变量快照推入 → JS 计算 → 收集 `_.set` 的变量更新 → 经 preview/patch 确认回写。

## 三、现状摸底（已核实）

`crates/infra-plugin-host/src/mvu_runtime.rs`：
- `MvuRuntime` trait（:51）已定义三个方法：`execute_fragment` / `load_card_assets` / `unload_card` + `is_available`。
- `StubMvuRuntime`（:83）全 NotImplemented，`is_available` 返回 false。
- 注释 :48-50 已列出实现候选：`WebViewMvuRuntime`（共享 WebView）、`QuickJsMvuRuntime`（未来可选）。
- `MvuExecResult`（:21）：`variable_updates: HashMap<String, Value>` + `side_effects: Vec<String>`。
- `execute_fragment(fragment_js, current_variables)` 是核心接口：输入 JS 片段 + 当前变量，输出变量更新。

**本 worktree 要做的**：实现 `WebViewMvuRuntime`，替换 `StubMvuRuntime` 作为上层默认（或可切换）。

## 四、JSR/ST API 常用子集 shim（已定范围）

JSR 脚本依赖的 API 分三层，shim 实现常用子集：

### 第一层：变量读写（核心，必须实现）
- `getChatVariable(key)` / `getChatVariable(key, default)` — 读变量
- `setChatVariable(key, value)` / `_.set(key, value)` — 写变量（收集到 MvuExecResult）
- `_.get(key)` — JSR 的 `JS_Slash_Star` 别名读
- `_.set(key, value)` — JSR 别名写

### 第二层：基础 DOM（常用，实现子集）
- `document.querySelector` / `document.querySelectorAll` — 作用于 shim 容器内 DOM
- `document.createElement` — 限定在容器内
- jQuery `$` 子集（`$.find`/`$.text`/`$.html`/`$.css`/`$.attr`）— 容器内操作
- `setTimeout`/`setInterval` — 受控（有上限，防死循环）

### 第三层：ST/JSR 事件与宏（常用子集，实现 + 不支持时降级 warn）
- `triggerSlashTag(command)` — 记录到 side_effects，不真执行 ST 命令
- `eventOn(name, handler)` / `eventOn('st_chat_changed')` — 注册回调（变量推入时触发）
- `registerSlashCommand(name, handler)` — 记录注册，命令触发时回调
- 不支持的 API（如 `fetch`/网络）→ shim 抛明确错误，上层 catch 后降级提示

### 不实现的（明确降级）
- 任意网络访问（fetch/XHR）— 默认禁用，shim 抛错
- 操作主 UI DOM（容器外）— shim 拦截
- JSR 的 iframe 跨域通信完整协议 — 只支持同容器内

## 五、实现拆解

### 任务 1：前端 WebView 容器 + shim（核心）

改动：新增 `frontend/src/components/MvuJsRuntime.vue`（隐藏容器）。

- 隐藏的 `<iframe>` 或 `<div>`，srcdoc 加载卡的 HTML + 注入 shim + 卡的 JS。
- shim 层（一段 JS）注入到容器，实现上述 API 子集。
- `_.set`/`setChatVariable` 的写入收集到一个 JS 对象，通过 postMessage 回传给主前端。
- 变量快照：主前端通过 postMessage 把 `current_variables` 推入容器，触发卡的 `eventOn('st_chat_changed')` 回调。

### 任务 2：Rust 侧 `WebViewMvuRuntime` 桥接

改动：`crates/infra-plugin-host/src/mvu_runtime.rs` 新增 `WebViewMvuRuntime`。

- `WebViewMvuRuntime` 持有一个 channel（mpsc）与前端容器通信：
  - `execute_fragment` → 发送 JS + 变量快照到前端 → 等待前端 postMessage 回传 `MvuExecResult`。
  - `load_card_assets` → 发送 html/css/js 到前端容器加载。
  - `unload_card` → 通知前端清空容器。
- `is_available` → true。
- 通信机制：Tauri event（`emit`/`listen`）或 invoke。前端 `MvuJsRuntime.vue` 监听 Rust 事件，执行后 invoke 回传结果。
- 注意：`execute_fragment` 是 async 的（trait 方法签名是同步的 `Result`——**需把 trait 改 async 或用阻塞等待**）。读 trait 签名决定：若 trait 是同步，可能要把 `execute_fragment` 改 `async fn`（影响所有实现 + 调用点），或用同步 channel 阻塞等（tokio runtime 内危险）。

⚠️ **trait 异步化是关键决策**：`MvuRuntime::execute_fragment` 当前是同步 `Result`，但 WebView 通信必然异步。要么 trait 改 async（影响调用点），要么用 `tokio::runtime::Handle::block_on`（有 runtime 嵌套风险）。**建议 trait 改 async**，顺带改调用点。

### 任务 3：接入上层（替换 Stub）

改动：上层（postprocess / pipeline）默认用 `WebViewMvuRuntime`，`fallback_fragments` 非空时调 `execute_fragment`。

- 读现有上层怎么用 `StubMvuRuntime`（grep `MvuRuntime`/`StubMvuRuntime` 调用点）。
- 替换为 `WebViewMvuRuntime`，`is_available` 为 false 时降级提示（前端显示"需 WebView JS 支持"）。
- JS 计算结果 `MvuExecResult.variable_updates` 经现有 preview/patch 确认回写，**不直接写 store**（红线）。

### 任务 4：测试

- 确定性：shim 的 `_.set`/`getChatVariable` 收集逻辑（纯 JS 单测，若有前端测试框架）；`WebViewMvuRuntime` 通信 mock。
- `cargo test --workspace` 0 回归（trait 异步化影响调用点，仔细改）。
- 真实 JS 卡验证（可选，需有 JS fallback 的 MVU 卡 fixture）：跑一轮，看 JS 计算的变量更新经 patch 回写。

## 六、关键约束

- **必须 WebView runtime，不要 QuickJS**（JSR 脚本依赖 DOM）。
- **JSR 兼容只做常用子集**（用户已定，不追完整——完整是黑洞）。
- **JS 输出必须经 preview/patch 确认，不直接写 store**（红线，PLAN-PLUGIN-MVU 禁止）。
- **JS 失败不影响主写作**（catch 后降级，写作继续）。
- **不执行任意远程 JS**（只跑卡内本地 JS，网络默认禁用）。
- **不碰 W7 的文件**（导出相关）。
- **不 commit**（留给用户审）。
- trait 异步化影响所有调用点，仔细改 + cargo test 全绿。

## 七、给 Claude Code 的提示词

```
请阅读 docs/PLAN-PLUGIN-MVU.md 阶段5、docs/HANDOFF-W8-JSRUNTIME.md（本文件），
然后实现 JS fallback WebView runtime + js-slash-runner 常用子集 shim。

工作目录：C:\Users\Predator\ZCodeProject\storyforge-w8-jsruntime
分支：w8-jsruntime

先读 crates/infra-plugin-host/src/mvu_runtime.rs 理解 MvuRuntime trait(:51) +
StubMvuRuntime(:83) + MvuExecResult(:21)。grep MvuRuntime/StubMvuRuntime 找上层调用点。

技术路径（必须）：WebView runtime，不要 QuickJS——JSR 脚本依赖 DOM。
用户已定范围：JSR/ST API 常用子集（不追完整）。

任务:
1. 前端新增 MvuJsRuntime.vue 隐藏容器（iframe srcdoc 加载卡 HTML+JS）+ shim 注入。
   shim 实现常用子集:
   - 变量读写: getChatVariable/setChatVariable/_.get/_.set(_.set 收集到 MvuExecResult)
   - 基础 DOM: querySelector/createElement/jQuery $ 子集/setTimeout(受控)
   - ST/JSR 事件: triggerSlashTag(记 side_effects)/eventOn/registerSlashCommand
   - 不支持: 网络访问默认禁用抛错/操作容器外 DOM 拦截
   变量快照 postMessage 推入,结果 postMessage 回传。
2. Rust 侧 mvu_runtime.rs 新增 WebViewMvuRuntime,通过 Tauri event/invoke 与前端通信。
   ⚠️ execute_fragment 当前同步,WebView 通信必然异步——建议 trait 改 async fn,
   仔细改所有调用点。load_card_assets/unload_card 同理。
   is_available 返回 true。
3. 上层替换 StubMvuRuntime 为 WebViewMvuRuntime,fallback_fragments 非空时调
   execute_fragment。结果经现有 preview/patch 确认回写,不直接写 store。
   JS 失败 catch 降级,不影响主写作。
4. 测试: cargo test --workspace 0 回归(trait 异步化仔细改)+ shim 收集逻辑单测。

红线: 必须 WebView 不用 QuickJS / JSR 只做常用子集 / JS 输出经 preview/patch 不直接写 store /
JS 失败不影响主写作 / 不执行远程 JS / 不碰导出文件 / 不 commit。

先做前端容器+shim(核心),再 Rust 桥接,再接入上层,每步跑 cargo check/npm run build。
```

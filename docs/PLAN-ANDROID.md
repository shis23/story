# 计划：Android 可用性打磨

> 状态：待执行
> 前置：Campaign 主流程在桌面端可稳定完成。

## 目标

让 Android 端能完成 StoryForge 主流程：导入 ST 卡、创建 Campaign、写作、查看状态、导出排障信息。

## 非目标

- 不手工大改 Tauri 生成的 Android 工程，除非验证证明必须。
- 不先做 Android 专属功能。
- 不在 Campaign 主线未稳定前追求移动端完美体验。

## 当前事实

- Tauri v2 配置在 `crates/tauri-app/tauri.conf.json`。
- capabilities 在 `crates/tauri-app/capabilities/default.json`，当前权限较粗：`core:default`、`fs:default`、`dialog:default`。
- Android 生成工程存在于 `crates/tauri-app/gen/android`。
- `AndroidManifest.xml` 只有 `INTERNET`、FileProvider、MainActivity。
- `MainActivity.kt` 只调用 `enableEdgeToEdge()`。
- 前端当前以桌面/窄窗口为主，已有大量弹层。

## 阶段 1：构建链路基线

目标：确认当前 Android 能否构建，不先修业务。

操作：

1. 记录本机 Android SDK/NDK 环境。
2. 运行前端构建：

```bash
cd frontend
npm run build
```

3. 运行 Rust 基线：

```bash
cargo test --workspace
```

4. 尝试 Android 构建：

```bash
cd crates/tauri-app
cargo tauri android build
```

验收：

- 形成 Android 构建记录。
- 如果失败，分类为环境问题、Tauri 配置问题、Rust 交叉编译问题、前端构建问题。

## 阶段 2：文件导入路径验证

目标：确认 Android 上 ST PNG/JSON 卡能通过系统选择器导入。

改动文件可能包括：

- `frontend/src` 导入相关组件
- `crates/tauri-app/src/lib.rs` 导入 command
- `crates/tauri-app/capabilities/default.json`
- 必要时 `crates/tauri-app/gen/android/app/src/main/AndroidManifest.xml`

测试用例：

- PNG 角色卡。
- JSON 角色卡。
- 大文件角色卡。
- 文件名含中文。
- 从 Downloads、Documents、第三方文件管理器选择。

验收：

- 成功导入后数据写入 app data，而不是依赖临时 URI。
- 失败时前端能显示可理解错误。
- 不要求泛读整个文件系统，只使用用户选择授权。

## 阶段 3：本地数据目录和迁移策略

目标：确认 Android app data 下 JSON store 可读写、升级不丢数据。

改动文件：

- `crates/tauri-app/src/lib.rs`
- store 初始化相关文件
- 日志路径相关文件

任务：

1. 打印或暴露 debug command：当前 data dir、log dir。
2. 验证以下文件在 Android 可创建：
   - `characters.json`
   - `cards.json`
   - `campaigns.json`
   - `instances.json`
   - `knowledge.json`
   - `tasks.json`
   - `round_summaries.json`
   - `mvu_translations.json`
3. 设计简单 schema/version 字段，至少记录 app version。
4. 写入失败时不要 panic，返回前端错误。

验收：

- 创建 Campaign 后重启 app 数据仍在。
- 存储错误可导出日志。

## 阶段 4：长任务、流式输出和取消

目标：Android 上写作流式输出可用，取消有效，断网可恢复到明确状态。

改动文件：

- `frontend/src/App.vue`
- `frontend/src/components/PipelinePanel.vue`
- `crates/tauri-app/src/lib.rs`
- `crates/infra-llm` 如涉及网络错误分类

测试场景：

- 写作中锁屏/切后台/回前台。
- 写作中取消。
- 网络断开。
- LLM 返回慢。
- 生成超长文本。

验收：

- cancel 后后端任务停止，前端不再继续追加 token。
- 错误状态不会留下无法关闭的 loading。
- 长文本不明显卡顿。

## 阶段 5：移动端排障导出

目标：Android 出问题时能拿到足够信息。

改动文件：

- `crates/app-logging`
- `crates/tauri-app/src/lib.rs`
- `frontend/src/components/LogPanel.vue` 或新增导出入口

任务：

1. 增加“导出诊断包”命令。
2. 包含：
   - app version
   - platform
   - recent logs
   - store 文件摘要，不含 API key
   - active campaign id
   - 最近一次 pipeline event 摘要
3. Android 用系统 share/save sheet 导出。

验收：

- 用户能从 Android 发出诊断包。
- 包内不包含 LLM API key。

## 阶段 6：权限收敛

目标：把 capability 从粗放权限收敛到最小可用权限。

改动文件：

- `crates/tauri-app/capabilities/default.json`
- 前端文件访问调用点

任务：

1. 盘点实际使用的 Tauri plugins。
2. 将 `fs:default` 收窄为 app data/log/import/export 所需范围。
3. dialog 权限只保留 open/save 所需能力。
4. Android 专属权限只在必要时加入 manifest。

验收：

- 导入、写作、导出诊断包仍可用。
- capability 文件中没有无理由的全量文件系统访问。

## 禁止改动

- 禁止手工重写整个 `gen/android`。
- 禁止把 API key 写入诊断包。
- 禁止为了 Android 临时绕过 Tauri capability。
- 禁止在移动端引入和桌面不同的数据模型。


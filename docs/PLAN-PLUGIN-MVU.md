# 计划：插件与 MVU 状态栏

> 状态：已基本实现（2026-06-19，MVU 状态栏/变量 schema/JS fallback/apply 前端均已落地；插件权限分层 MVU-6 部分未覆盖）。
> 前置：Campaign 主线已稳定。

## 目标

先让 SillyTavern/MVU 状态栏能力进入 StoryForge 的 Campaign 运行态，再考虑更通用的插件系统。

## 非目标

- 不先做插件市场。
- 不先做任意第三方 JS 的完整执行沙箱。
- 不让插件直接改写 CampaignStore。
- 不把 MVU 运行结果绕过 patch/preview 直接写入变量。

## 当前事实

- `crates/infra-plugin-host/src/lib.rs` 已有 `PluginManifest`、`PluginRegistry`、权限、安装/启用/卸载能力。
- `crates/infra-plugin-host/src/mvu_runtime.rs` 定义纯 `MvuRuntime` trait/DTO/事件协议；`StubMvuRuntime` 保留为 harness/降级实现，Tauri/WebView 可用实现位于 `crates/tauri-app/src/mvu_webview_runtime.rs`。
- `crates/app-meta/src/mvu_import.rs` 已能分析 MVU 卡，产出 `MvuTranslation`。
- `crates/tauri-app/src/lib.rs::meta_analyze_mvu_card` 会保存 `StoredMvuTranslation`。
- `frontend/src/components/MetaPanel.vue` 能触发 MVU 分析和查看 translation。
- `frontend/src/components/MvuStatusBar.vue` 已存在，但需要确认与 Campaign variables 的闭环。

## 阶段 1：冻结通用插件扩张

目标：先定边界，避免插件系统抢主线资源。

改动文件：

- `docs/PLAN-PLUGIN-MVU.md`
- 如需要，`docs/ROADMAP.md`

任务：

1. 明确短期插件只服务三类能力：
   - MVU 状态栏 schema 导入
   - 状态栏渲染
   - 必要 JS fallback 执行
2. 不新增插件市场、远程安装、动态权限弹窗。
3. PluginPanel 保持 manifest 管理即可。

验收：

- 开发计划中没有绕过 Campaign 的泛插件任务。

## 阶段 2：MVU Translation -> Variable Schema Preview

目标：把 MVU 分析产物转成可审阅 schema diff。

改动文件：

- `crates/domain/src/mvu_translation.rs`
- `crates/domain/src/variables.rs`
- `crates/app-meta/src/mvu_import.rs`
- `crates/tauri-app/src/lib.rs`
- `frontend/src/components/MetaPanel.vue`

任务：

1. 为 `MvuTranslation.variable_schema` 生成 schema diff：
   - 新增字段
   - 重名字段
   - 类型冲突
   - 默认值冲突
2. 增加 command：
   - `meta_preview_mvu_schema(source_character_id, campaign_id)`
3. 前端展示 diff，不直接应用。

验证：

```bash
cargo test -p storyforge-domain
cargo test -p storyforge-app-meta
cargo test -p storyforge
```

验收：

- 用户能看到 MVU 会新增/修改哪些变量字段。
- 冲突字段不会静默覆盖。

## 阶段 3：Apply MVU Schema Patch

目标：用户接受后，把 MVU schema 合并进 Campaign 对应角色。

前置：

- `PLAN-META-AGENT.md` 阶段 3 Typed Patch Preview。

改动文件：

- `crates/app-meta/src`
- `crates/tauri-app/src/lib.rs`
- `crates/domain/src/character.rs`
- `crates/domain/src/campaign.rs`

任务：

1. 新增 patch action：`ApplyMvuVariableSchema`。
2. target 必须是 `campaign_id + character_definition_id` 或 `instance_id`。
3. 应用后：
   - definition schema 合并字段。
   - 现有 instance variables 初始化缺失默认值。
4. 保留原始 `StoredMvuTranslation`，便于回看。

验收：

- 接受 patch 后 CampaignPanel 能看到新变量。
- 下一轮写作可注入这些变量。

## 阶段 4：状态栏原生渲染

目标：对 pure data / native routing 的 MVU，先用 Vue 原生组件渲染状态栏。

改动文件：

- `frontend/src/components/MvuStatusBar.vue`
- `frontend/src/components/CharacterDetail.vue`
- `frontend/src/components/CampaignPanel.vue`
- `frontend/src/tauri-api.js`

任务：

1. 从 Campaign variables 读取状态，不从原始 ST extension 直接读。
2. 支持字段类型：
   - int/float：数字或进度条
   - bool：开关/状态点
   - string：标签
   - json：折叠详情
3. 只读展示优先，手动编辑走 Campaign variables API。

验收：

- MVU 状态栏显示的是当前 Campaign 变量值。
- 写作后变量变化，状态栏刷新。

## 阶段 5：Hybrid JS fallback Runtime

目标：只为必须执行 JS 的卡提供受控 fallback，不做通用插件沙箱。

改动文件：

- `crates/infra-plugin-host/src/mvu_runtime.rs`
- `crates/tauri-app/src/lib.rs`
- `crates/tauri-app/src/mvu_webview_runtime.rs`

候选实现：

- 共享 WebView runtime：贴近 ST 行为，但隔离和调试复杂。
- QuickJS runtime：更可控，但 DOM/CSS 能力弱。

执行规则：

1. runtime 输入是变量快照和声明式 interaction。
2. runtime 输出是候选 variable updates。
3. 输出必须经过 preview/patch 或明确用户动作确认。
4. runtime 不直接持有 CampaignStore。

验收：

- `StubMvuRuntime` 已可由 Tauri 层 `WebViewMvuRuntime` adapter 替换。
- JS fallback 失败不影响主写作。
- 错误可显示在 Meta/MVU 面板。

## 阶段 6：Plugin 权限与安全清单

目标：在扩展通用插件前先定安全边界。

任务：

1. Manifest 权限分级：
   - read_campaign
   - read_variables
   - propose_variable_update
   - render_panel
   - network_access
2. 插件不能直接写 store，只能 propose patch。
3. 插件 UI 和主 UI 数据通道必须有类型校验。
4. 网络访问默认关闭。

验收：

- 插件权限模型能解释每个现有能力。
- 没有“安装即全权限”的路径。

## 禁止改动

- 禁止插件直接拿 `CampaignStore`。
- 禁止执行任意远程 JS。
- 禁止绕过 Meta patch preview 修改变量。
- 禁止为了少数复杂 ST 卡牺牲 native/pure data 路径。

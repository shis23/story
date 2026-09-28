# R7 跨域接缝验证（第二遍审查）

- 任务：task-26（owner `review-meta-plugin`）；代码冻结期**只读**验证 + 只写本报告。
- 环境：门禁已由 Lead 收口（`docs/review-2026-09-13/fixes/GATE-REPORT.md`：fmt=0、clippy=0、Rust 99 套件 2165 passed/0 failed/33 ignored、`npm test` 532/532、vitest 31 文件 151 全绿、`backend-baseline.mjs`=0、Pester 198/0）。
- 判定口径：**接缝对上 / 只对上一边（附反证） / 未验证**；新发现编号 `N-R7-xx`（file:line + 严重度）。
- 计分：**对上 6 / 只对上一边 1 / 未验证 1**；新发现 **5**（P2×1、P3×2、P4×2）。

## 1. W-01 身份归一 —— 只对上一边（反证见下）

| 侧 | 证据 |
|---|---|
| domain 匹配语义 | `crates/domain/src/campaign_runtime.rs:108`：`.flat_map(|inst| [inst.id.as_str().to_lowercase(), inst.name.to_lowercase()])` —— **Unicode `to_lowercase()`，name 不 `trim`**；`:112-118` id 侧 `trim()` + `to_lowercase()` 去重 |
| app-agent 消费侧 | `crates/app-agent/src/runtime.rs:608`：`candidate.trim().eq_ignore_ascii_case(needle.trim())` —— **trim + ASCII-only 大小写折叠**；`:615 find_instance_normalized` 被 `:700` 用于 `task.character_id` 解析 |

**反证（同一输入两侧结论不同）**：名字 `"Ähre"` vs `"ähre"` —— domain 的 `to_lowercase()` 判等（Unicode），app-agent 的 `eq_ignore_ascii_case` 判不等（非 ASCII 不折叠）；名字 `" Alice "` vs `"Alice"` —— domain 的 name 键含空白而不匹配，app-agent trim 后匹配。`runtime.rs:604` 的注释自称"`trim + to_lowercase`，此处必须保持同一套语义"——**实现与注释不符**，两侧不是同一套归一。

- 最小验证：`find_instance_normalized` 的单元测试若只用 ASCII 名 + 无首尾空白（现有用法），差异不会被覆盖；建议加 `"Ähre"` 与 `" Alice "` 两例跨侧断言。
- 影响：非 ASCII 角色名或带首尾空白的名字在 app-agent 侧无法解析到实例（可能落回 name→instance 兜底或 `fallback_reason`），与 domain 索引口径不一致。
- **N-R7-01（P3）**：`crates/app-agent/src/runtime.rs:604-608`（注释与实现不一致，跨侧归一语义分歧）。

## 2. D-04 StoryTime 注入 —— 对上

- 触发语义：`crates/domain/src/story_task.rs:145 check_trigger(current_turn, story_clock)` + `:256 normalize_story_clock`；`:278 render_tasks_for_injection(..., story_clock)`。
- 传参侧：`crates/app-pipeline/src/turn_dossier.rs:149` 注释明确"`story_clock`：真实故事时钟（D-04，**不得传空串**，否则 StoryTime 触发器恒不命中）"、`:154` 形参与 `:289` 透传。
- 旧数据：`normalize_story_clock` 对空/缺失走归一默认，dossier 侧有 `story_time_task_without_clock_goes_to_pending_judgment_group`（`:484`）与 `story_time_task_injected_only_with_matching_real_clock`（`:439`）两条测试，覆盖"不误触发 + 无时钟进待判断组"。
- 判定：**对上**（形参→触发语义→测试三处闭环）。

## 3. M-04 变量命名空间守卫 —— 对上（Rust 为权威且更严）

- Rust 侧（真拒绝，不是只在前端拦）：`crates/app-pipeline/src/lib.rs:4049-4051` `is_reserved_mvu_key` = `key.trim().to_ascii_lowercase().starts_with("__storyforge")`，在 `:4017 push_mvu_js_variable_updates` 中跳过并计 `skipped`；另 `crates/domain/src/typed_patch.rs`（`RESERVED_VARIABLE_NAMESPACE`）与 `crates/app-meta/src/typed_patch.rs` `validate_variable_write_key`（先 `normalize_mvu_key` 再判空前缀）各自独立设防。
- 前端镜像：`frontend/src/utils/shellVariableOutbox.js:16` `key.startsWith('__storyforge')`、`:106` 拆分后再复核（防 `instance:foo:__storyforge_x` 伪装）。
- **口径差异（方向安全）**：前端只做**精确前缀**匹配（大小写/空白不归一），Rust 做 `trim + ASCII 大小写不敏感`。故 `__StoryForge_x` / `" __storyforge_x"` 会通过前端 outbox，再由 Rust 丢弃 ⇒ **防线成立**，但调用方看到的是"写入静默消失"（无错误回传）。
- **N-R7-02（P3）**：前端镜像比 Rust 宽松（`shellVariableOutbox.js:16`），变体键会静默丢失，建议同口径归一或返回显式拒绝。
- **N-R7-03（P4）**：`frontend/src/utils/mvuExecuteResult.js:5` 注释仍写"该 Rust 侧守卫由另一 owner 补，这里先在前端镜像拒绝"——守卫**已落地**（`app-pipeline/lib.rs:4049`），注释过时。

## 4. M-08 prompt hook 权限 —— 对上

- 后端契约：`crates/tauri-app/src/commands/plugins.rs:258-259` 明确"prompt hook 是广播给宿主的，最终 messages 可能由多个插件依次改写，所以真实契约是**改写者集合**；单值 `plugin_id` 作为兼容兜底"；未提供任何有权限改写者时 `prompt_mutation_denial` 返回拒绝（消息被丢弃而非放行）。
- 前端调用点：`frontend/src/composables/usePluginBridge.js:303` 建数组 → `:316` `pluginPromptHookResult(requestId, messagesForBackend, null, { modifierPluginIds: appliedPluginIds })`；`frontend/src/utils/promptHooks.js:531-532` **只在 mutation 被采纳后** push id（不传该 option 时行为与旧版一致）；`tauri-api.js` wrapper 已透传。
- 不传时是否拒绝：**拒绝**（fail-closed），由 `prompt_hook_mutation_requires_a_permissioned_modifier_plugin`（`crates/tauri-app/src/lib_tests_meta.rs`）覆盖全拒绝路径 + 两条放行路径。
- 判定：**对上**。

## 5. T-01 契约扫描 ↔ 裸 `_invoke` ↔ `retainedNoFrontendCaller` —— 对上

- 实测：`node scripts/architecture/backend-baseline.mjs` → **exit 0**，末行 `[backend-baseline] 定义 175 / 注册 175 / 前端唯一 invoke 152 / 孤儿命令 23 (...)`，与 Lead 门禁数字一致；23 条孤儿在输出中逐条列出（`abandon_turn`、`add_world_info_entry`、… `update_world_info_route`）。
- 三集合关系成立：定义 175 == 注册 175；前端唯一 invoke 152 + 孤儿 23 == 175；孤儿清单 == 声明保留清单（含 CLAUDE.md 记载的 `delete_character` 保留依据）。
- 判定：**对上**（本项同时是 R7 里唯一可完全自证的整链断言）。

## 6. S-01 fail-closed ↔ `storage_backend.rs` cutover 决策路径 —— 未能确认（证据不足）

- 已确认存在：`crates/tauri-app/src/storage_backend.rs` 的 `StorageFacade`（`:187 new`、`:209 backend`、`:213 is_sqlite`、`:227 has_json_writers`、`:234 capability`、`:268 require_supported`、`:285 validate_runtime_authority`）与 cutover 相关导入（`:26-27 CutoverOutcome/Plan/Request/…`）；消费面可见 `backend_workflows.rs:34/2180`、`card_studio_api.rs:178/780` 使用 `StorageFacade`/`BackendCapability`。
- **未取得**"新 readiness 判据被 app **启动路径**使用（而非只在死分支/测试辅助）"的调用链证据：我用 `storage_backend::*`、`backend_readiness`、`cutover_decision` 等符号在 `crates/tauri-app/src/*.rs` 检索未命中启动装配点（`lib.rs` / `setup`）的直接调用。
- 判定：**未验证**（不写成"对上"，也不写成"未对上"——缺证据）。
- **N-R7-04（P2）**：需 S-01 owner 给出 `storage_backend.rs:268/285` 在启动路径（`lib.rs` 装配或 setup）的调用点 file:line + 一条"readiness 为负时启动拒绝/降级"的测试；最小验证：`cargo test -p storyforge --lib -- storage_backend` 观察是否存在启动路径断言。

## 7. "降级方案"残留风险是否显式记录 —— 对上（抽查本域 + 邻域）

- 本域 M-13 降级残点（`Create{world_info}` 静默 no-op）已在 task-25 **修复**并留测试；M-01 主体**暂缓**且 §3 记录 3 个候选真修复 + 否决 ACL 方案的源码依据；M-17 由"已修复"改为"**已修复 + 暂缓（需产品决策）**"，§12 第 6 条给出候选 A/B 与各自代价；M-24 的三条取舍（自导航撤信、hash-only 不可重新握手、不缓解 M-01）写在 §10 S3 行与 §14。
- 邻域：域5 记录显式标注 harness 能力限制（`05-frontend-fixes.md:25/411/426`）；域1 记录标注 M-32.9 采用"同步而非剔除"的口径理由。
- 判定：**对上**，未发现"降级被静默继承"的条目。

## 8. 额外接缝（Lead 指定）：PluginHost M-24 令牌时序 ↔ 测试 ↔ 记录 §14/§15 —— 对上

- 组件：`frontend/src/components/PluginHost.vue` 现为 `composeShellDoc(plugin, token)` 纯函数 + `handshakeSourceKey`（只依赖 `plugin.id`/`entry_html`）+ 回调"先生成令牌、再组装、再注册"，令牌不再是 watch 源依赖（§14.1 记录与代码一致）。
- Lead 收口两处已在 §15.2 如实记为「**Lead 收口修复**」（非本域成员产物）：`expectedFrameLoad`（额外 load 轮换令牌，且明确"不能无条件轮换"）与 `stripStyleElementsFromSlotHtml`（sanitize 前整块删 `<style>`）。
- 测试：`plugin-host-handshake.test.mjs` 新增 `registers exactly once per source change and embeds the token it later accepts (M-24 invariant)`；真实门禁 31 文件 / 151 全绿（Lead）。
- **环境事实未被误用**：§15.3 把"happy-dom 下 DOMPurify 3.4.12 默认白名单解析为空（`sanitize('<p>hi</p>')==='hi'`，`isSupported===true`）"写为**环境限制**，并明确"元素/属性会被保留"的断言在 happy-dom 下不可靠。检索 `docs/review-2026-09-13/fixes/*.md` 未发现任何条目把它当作"代码行为已在真 Chromium 验证"；相反，域5/域6 都把 harness 覆盖不到的点标为"待 Lead 门禁"。
- **N-R7-05（P4）**：域5 记录 `05-frontend-fixes.md:426` 仍写"tavern-helper 剩余 2 条断言**组件文本**含 iframe/宿主相关串"——该探针在 task-29 已改为"宿主对帧的**出站 postMessage 回复**"，域5 记录未同步（跨域文档漂移，不影响代码）。

## 9. 新发现汇总

| 编号 | file:line | 严重度 | 摘要 |
|---|---|---|---|
| N-R7-01 | `crates/app-agent/src/runtime.rs:604-608` | P3 | 注释自称与 domain 同一套 `trim+to_lowercase` 归一，实现为 `trim+eq_ignore_ascii_case`；非 ASCII 名/带空白名两侧判等不一致（`campaign_runtime.rs:108`） |
| N-R7-02 | `frontend/src/utils/shellVariableOutbox.js:16` | P3 | 前端保留命名空间判定只有精确前缀（不比 Rust 的 `trim+ASCII 大小写不敏感`），变体键静默丢失（防线仍成立，Rust 兜底） |
| N-R7-03 | `frontend/src/utils/mvuExecuteResult.js:5` | P4 | 注释仍称"Rust 侧守卫由另一 owner 补"，实际已落地（`app-pipeline/src/lib.rs:4049`） |
| N-R7-04 | `crates/tauri-app/src/storage_backend.rs:268,285` | P2 | 未取得 readiness 判据在**启动路径**被消费的证据（S-01 接缝未验证，需 owner 补 file:line + 测试） |
| N-R7-05 | `docs/review-2026-09-13/fixes/05-frontend-fixes.md:426` | P4 | 域5 记录未同步 task-29 对 tavern-helper 探针的更改（出站回复取代组件文本） |

**本轮未做的事**：未跑 `cargo test --workspace`（Lead 独占）、未跑 npm（沙箱 EPERM）、未修改任何源码/文档（只写本报告）。

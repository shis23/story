# StoryForge 全量审查复核报告

> 审查日期：2026-09-13　|　审查基线：`main @ ab894c6`（工作树干净）　|　版本：0.1.2
> 组织方式：Agent Teams（1 Lead + 7 名分域 teammate，全程只读审查 + Lead 独立复核）
> 本报告为最终汇总；7 份分域原始报告见 `docs/review-2026-09-13/01..07-*.md`，是每条发现完整证据（代码摘录）的出处。
> 完整性声明：本次审查**未修改任何源码、测试、配置或既有文档**（`git status --porcelain` 仅 `?? docs/review-2026-09-13/`）。

---

## 0. 执行摘要（TL;DR）

**一句话结论：StoryForge 是一个"工程质量明显高于同体量项目平均线"的成熟项目——v0.1.2 的发布范围基本达成且可独立复现，核心写作链路与 SQLite 提交链路经得起逐行审查；但本次全量审查仍挖出 3 个 P0 与 44 个 P1，其中贯穿全部 7 个域的头号系统性缺陷是「失败被静默吞掉」。**

| 维度 | 结论 |
| --- | --- |
| 预期目标完成度 | **39 项核对：达成 29 / 部分达成 5 / 未达成 4 / 无法判定 1**。未发现"把未达成写成已达成"的虚假验收；未达成的 4 项中有 2 项是**如实标注的未完成**（JSON 生产写路径删除、CoT 三臂），2 项是**收尾后未回写的陈旧声明**（README:25、RELEASE-STATUS 门禁表第 1 行） |
| 代码逻辑正确性 | 主链路成立（Turn/Accept/SQLite UoW/CAS/幂等/隔离骨架），但存在 **3 个 P0**：卡壳解析 panic、Windows 子帧 IPC 暴露、legacy 迁移静默全量丢数据 |
| 冗余与死代码 | 真实存在且规模不小：**18 个零引用/仅测试引用前端组件**、**21 个孤儿 wrapper**、**3 个孤儿后端命令**、`tool_center.rs` 死代码、`components-v2/writing/**` 整树死代码 |
| 前端组件可用性 | 主流程可用、无 P0；问题集中在"失败不可见"（吞错 / 空态假象 / 假保存）与"实现但未接线"。**中文输入法回车误提交（F-01）是本项最该先修的一条** |
| 测试与证据 | 确定性门禁**全绿且计数与文档完全一致**（本次 Lead 独立复跑）；但护栏的"宣称强于实际"：3 个 live 命令调用点对契约测试不可见、Card Studio 落盘错误无测试拦截、真实卡保真测试全部 `#[ignore]` 且 fixture 是手工桩 |
| 本次发现总量 | **P0 = 3　P1 = 44　P2 = 100　P3 = 47　合计 194 条**（全部 P0 与 12 条 P1，共 **15 条经 Lead 回到源码/运行复核**，其余为域内自核；未复核项已在第 8 节列明） |

**最该先做的 6 件事（按性价比排序）**：
1. 修 **P0-1** 卡壳解析 UTF-8 panic（一行守卫，Lead 已用隔离 rustc 复现）。
2. 修 **P0-3** 迁移 fail-closed（`cards.json` 缺失时不得把全部 Campaign 当孤儿丢弃）。
3. 跑 **P0-2** 的 30 秒运行时 PoC 并修 Windows 子帧 IPC/ACL（若成立，等于插件与卡壳权限体系整体失效）。
4. 修 **W-01** 名字归一不一致（大小写/空白变体让子 Agent 失去 instance 绑定，信息隔离降级）。
5. 修 **T-01 + T-02**（契约测试盲区 + 8 处吞掉落盘错误），改动小、直接恢复护栏。
6. 修 **F-01 + F-02**（IME 误提交 + 采纳失败零反馈），用户可直接感知。

---

## 1. 审查方法与证据等级

### 1.1 组织方式

7 个互不重叠的审查域并行执行（写作用域按文件隔离，每域仅可写自己的报告文件），Lead 负责：门禁复跑、跨域裁决、P0/P1 复核、汇总。

| 域 | 负责人 | 范围 | 报告 |
| --- | --- | --- | --- |
| 1 | review-domain | `domain` + `infra-util/vector/regex` | `01-domain-infra.md` |
| 2 | review-storage | `infra-sqlite` + tauri-app 存储/Turn 生命周期 | `02-storage-lifecycle.md` |
| 3 | review-pipeline | `app-agent/pipeline/conversation/memory` | `03-writing-pipeline.md` |
| 4 | review-tauri-api | 175 命令 / DTO / 错误 / 前后端契约 | `04-tauri-commands.md` |
| 5 | review-frontend | 前端通用层（组件可用性/状态/冗余/契约） | `05-frontend.md` |
| 6 | review-meta-plugin | Meta/MVU/插件/Card Shell/导入导出 | `06-meta-mvu-plugin.md` |
| 7 | review-goals | 文档声明 / 门禁 / harness / CI / 证据 | `07-goals-and-claims.md` |

### 1.2 Lead 独立门禁复核（本次实测，非引用文档）

| 门禁 | 命令 | 结果 | 与文档声称 |
| --- | --- | --- | --- |
| 格式 | `cargo fmt --all -- --check` | **通过**（exit 0） | 一致 |
| 严格 Clippy | `cargo clippy --workspace --all-targets -- -D warnings` | **通过**，0 warning（exit 0） | 一致 |
| Rust 工作区测试 | `cargo test --workspace` | **98 套件：1980 通过 / 0 失败 / 33 忽略**（exit 0） | **完全一致**（文档 1980/0/33） |
| 前端逻辑 | `npm test`（node --test） | **504 通过 / 0 失败 / 0 跳过**（exit 0） | 一致 |
| 前端组件 | `npm run test:ui`（vitest） | **27 文件 / 119 通过**（exit 0） | 一致 |
| 生产构建 | `npm run build` | **通过**（6.36s；有 >560KB chunk 警告） | 一致 |

> 未复跑项：Playwright 三套（CSP 9 / mobile 28 / smoke 2）。它们在文档中的 504+119+9+28+2=662 口径里占 39 项，本次未验证；且其中 `smoke:ui` 在缺依赖时只记录 skip。
> 环境限制：前端三套命令在受限沙箱下因 Node/esbuild 子进程管道 stdio 被 EPERM 阻断，Lead 经一次沙箱放宽后取得以上真实结果；这与项目代码无关，已排除为环境因素。

### 1.3 证据等级（本报告统一标注）

- **[A] Lead 已复核**：Lead 亲自回到源码（必要时运行隔离复现或脚本）逐条验证。本次共 **3 条 P0 + 12 条 P1 = 15 条**达到 A 级（清单见第 3、4 节的"复核"列）。
- **[B] 域自核**：分域负责人声称已读源码并给出 file:line 与摘录，Lead 未逐行复核。
- **[C] 疑似**：域负责人自己标注为需进一步确认（本报告已显式保留）。

### 1.4 方法与局限（诚实边界）

1. **全程静态审查为主**：未运行 GUI 端到端、未调用真实 LLM、未跑 Android/Windows 真机、未跑 Playwright。涉及"运行时行为"的结论均标注了证据等级。
2. **未覆盖**：`crates/harness-real-llm` 的真实模型入口（`#[ignore]`）、`gen/android`、`icons/`、`node_modules`。
3. **未逐行覆盖**：域1 明确未逐行复核 `turn.rs:130-780`；域2 的 WAL/多进程时序为疑似；域7 的 APK 签名与 Gitea 服务端状态无法本地验证。
4. **P0-2 的运行时 PoC 未执行**（需要装插件并观察 WebView），已给出 30 秒复现步骤（见 3.2）。
5. 分域报告存在 2 处**内部计数不自洽**，已由 Lead 裁决并在第 9.4 节记录。

---

## 2. 总体判定

### 2.1 预期目标完成度（域7 逐条核对 39 项）

| 判定 | 数量 | 说明 |
| --- | --- | --- |
| 达成 | **29** | ROADMAP 6 / README 7 / 门禁表 12 / ARCHITECTURE 2 / HANDOFF 2 |
| 部分达成 | **5** | ROADMAP Phase 6、门禁表"ST 世界书往返"、ARCHITECTURE 技术债 3/4、HANDOFF 第 4 项 |
| 未达成 | **4** | README:25（陈旧冲突）、门禁表第 1 行（文档一致性不成立）＝**陈述性缺陷**；HANDOFF 第 3/5 项（JSON 写路径删除、CoT 三臂）＝**如实标注的未完成** |
| 无法判定 | **1** | ROADMAP Phase 7（无状态标记、无判定；性能/成本项无专门证据） |

**如实标注的未完成项（不是缺陷，是"标注合格"）**：Gate 6 Full100（4 个 `run-full-*` 目录无 `run_manifest.json`，维持"关闭非 PASS"）、记忆参数未标定（`H_anchor=5`/`E=10` 为默认值）、CoT 三臂（只有 PLAN/PROMPT 无 RESULT）、JSON 生产写路径删除。

**core 发布结论可独立核实**：v0.1.2 tag/Release/三产物 digest 与文档哈希逐个一致（域7 经 GitHub API 复核）；Windows 原生 + Android 真机真实模型写作、第三方插件两条通道的来源证据在 `artifacts/` 与 `docs/workstreams/` 中存在。

### 2.2 分层质量评价

| 层 | 评级 | 依据 |
| --- | --- | --- |
| 命令层与注册 | **优** | 175 `#[tauri::command]` ↔ 175 `generate_handler!` 双向一致，无漏注册/重复；165 个 JS 调用站点仅 1 处参数漂移；生产路径 `unwrap` 近零（全 src 20 处，commands 仅 1 处不变量断言）；capability 最小权限用法正确 |
| SQLite 提交链路 | **优** | Accept UoW 原子性、事务内 CAS（expected/target/revision 三重）、ledger 幂等 replay、late attempt guard、Degraded 不可逆、失败传播到命令边界——逐行核对成立 |
| domain 模型 | **良** | 纯度达标（无 tauri/网络/IO 依赖）；CLAUDE.md/DATA_MODEL.md 的事实条目逐条一致，未发现声明造假；问题是边界与静默降级（1 panic、1 往返回滚、1 缺栅栏） |
| 写作流水线 | **良** | 四模式、QualityGate+有界 1×auto-fix、三件套、provenance、profile 参数消费均在真实路径；风险集中在身份归一不一致与 LLM 畸形输出时的静默降级 |
| 存储迁移/cutover | **中** | 主链路优，但"缺失=空 + 悬空过滤"可静默全量丢数据（P0-3），崩溃窗口与 fresh-start 身份判据有可用性/潜在数据风险 |
| Meta/MVU/插件/卡壳 | **中→差** | 数据面干净（PNG 保真、Bundle 外键、MVU apply 幂等），但安全边界存在 P0 级缺陷；文档声明高于实现 |
| 前端 | **中** | 主流程可用、无断链，但"失败不可见"成族，且有 18 个死组件 + 21 个孤儿 wrapper |
| 文档与声明体系 | **中** | 数量类事实全部可复现，未完成项标注诚实；但收尾后 4 份"当前"文档未回写，产生 5 条 P1 冲突 |

### 2.3 发现计数总表

| 域 | P0 | P1 | P2 | P3 | 小计 |
| --- | --- | --- | --- | --- | --- |
| 1 domain/infra | **1** | 3 | 12 | 10 | 26 |
| 2 存储与 Turn | **1** | 6 | 11 | 4 | 22 |
| 3 写作流水线 | 0 | 6 | 20 | 6 | 32 |
| 4 Tauri 命令层 | 0 | 3 | 9 | 4 | 16 |
| 5 前端 | 0 | 14 | 21 | 12 | 47 |
| 6 Meta/MVU/插件/卡壳 | **1** | 7 | 18 | 6 | 32 |
| 7 目标与声明 | 0 | 5 | 9 | 5 | 19 |
| **合计** | **3** | **44** | **100** | **47** | **194** |

---

## 3. P0 发现（3 条，全部经 Lead 复核）

### 3.1 P0-1｜卡壳 ES module URL 解析在中文卡上必然 panic　`[A] Lead 已实证复现`

- **位置**：`crates/domain/src/card_shell.rs:428`、`:441`（兄弟函数 `:351-354` 有正确的边界推进守卫）
- **缺陷**：`capture_es_module_urls` 在 `search_from = abs + 1` 后不推进到字符边界，下一轮 `lower[search_from..]` 在非边界处切片。
- **触发条件（Lead 隔离复现）**：卡内容中 `from ` 或 `import ` 后紧跟多字节字符，例如中文注释 `// 移植 from 原作：设定集`。
  ```
  [OK]    ASCII only: 1 urls
  [PANIC] CJK after from-space
  [PANIC] CJK after import-space
  start byte index 16 is not a char boundary; it is inside '原' (bytes 15..18)
  ```
- **可达性（已核实）**：`extract_from_tavern_helper`（`card_shell.rs:228`）→ `extract_card_shell_manifest`（`:101`）→ Tauri 命令 `get_card_shell_manifest`（`crates/tauri-app/src/commands/card_shell.rs:36`），输入是**不可信的导入卡 JSON**。
- **影响**：用户导入/打开一张含中文注释或中英混排文本的 TavernHelper 卡即可触发 panic；至少该命令失败，最坏中断 IPC 处理线程（项目未设 `panic=abort`，实际后果取决于 Tauri 的 panic 处理，但"由不可信输入触发 panic"本身即缺陷）。
- **修复建议**：`search_from = abs + 1; while search_from < text.len() && !text.is_char_boundary(search_from) { search_from += 1; }`（照抄同文件 `:351-354` 的既有模式），并补一条含中文的回归测试。
- **域报告**：D-01（`01-domain-infra.md`）。

### 3.2 P0-2｜Windows 子帧可直接 invoke 任意 Tauri 命令，权限桥整体绕过　`[A] 静态 4/5 环已复核，运行时 PoC 待执行`

- **位置（本仓库）**：`crates/tauri-app/src/shell_doc_protocol.rs:52-77`（CSP 无 `ipc:`）、`crates/tauri-app/capabilities/default.json`（仅 8 条权限、`windows:["main"]`）、`crates/tauri-app/` 无 `permissions/` 目录、`build.rs:18` 仅 `tauri_build::build()`（→ 无 app ACL manifest）
- **依赖侧证据（Lead 已打开 cargo registry 源码核对）**：
  - `wry-0.55.1/src/lib.rs:990`：「**Windows:** scripts are always added to subframes regardless of the `for_main_frame_only` option.」→ `__TAURI_INTERNALS__` 进入子帧；
  - `tauri-2.11.5/scripts/ipc-protocol.js:59-84`：自定义协议 IPC 被 CSP 阻断时**回退到 `window.ipc.postMessage`**（不受 CSP 约束）；
  - `tauri-2.11.5/src/webview/mod.rs:1744/1787/1823-1832`：`is_local_url` 对 `*.localhost` 自定义协议判为 local，且 ACL 校验在 `!plugin_command && !has_app_acl_manifest && is_local` 时被跳过。
- **链路**：卡壳/插件/MVU/TH iframe（`storyforge-shell://localhost` 或 `*.localhost` 源）→ 获得 `__TAURI_INTERNALS__` → CSP 挡自定义协议 IPC → 回退 `window.ipc.postMessage` → local 源无 app ACL → **可调用任意已注册命令**（`delete_conversation`、`install_plugin`、`meta_accept_typed_patch`、`get_conversation` 等）。
- **影响**：若成立，`permissions:[]` 的插件或恶意卡即可读全量记忆、删会话、装插件——插件权限桥、卡壳网络白名单、Meta 的"先 preview 再 accept"三道门同时失效。
- **Lead 已复核的边界**：CSP 无 `ipc:`、无 app ACL、wry/Tauri 的上述行为**均已在源码确认**；唯一未确认的是**运行时**：WebView2 是否实际把初始化脚本投递给 `sandbox="allow-scripts"` 子帧、以及 `window.ipc` 在该子帧是否可用。
- **30 秒运行时 PoC（请优先执行）**：装一个 `permissions: []` 的插件，`entry_html` 打印 `typeof window.__TAURI_INTERNALS__` 与 `typeof window.ipc?.postMessage`。若两者均非 undefined，P0 成立。
- **修复建议**：为 app 增加 ACL manifest（或用 `tauri.conf.json` 的 `app.security.capabilities` 显式约束子帧源）；CSP `connect-src` 增补 `ipc:` 以保留自定义协议通道并避免回退；对 `__TAURI_INTERNALS__` 注入做 `for_main_frame_only` 的可行替代（如在协议响应中校验 `Sec-Fetch-Dest`）；壳文档与插件 iframe 的 postMessage 桥加 `event.source` 归属校验（与 M-07 同源）。
- **域报告**：M-01（`06-meta-mvu-plugin.md`）。

### 3.3 P0-3｜legacy 树不完整时静默全量丢弃，并被权威 marker 永久固化　`[A] Lead 已复核链路`

- **位置**：`crates/infra-sqlite/src/readiness.rs:123-134`（全部核心集合 `optional = true`，缺失=空）、`:369-377`（`card_id` 不在源 cards 中的 Campaign **静默丢弃**，只计数不报错）、`:250-272`（manifest hash 在**过滤之后**计算，故空投影自洽）、`crates/infra-sqlite/src/cutover.rs:395-417`（`legacy_json_layout_present` 只要有任一 legacy 文件即走正常 cutover）、`cutover.rs:1050-1057`（marker-last 发布）、`crates/tauri-app/src/storage_backend.rs:1884`（`skipped_orphan_rows` 报告被整体丢弃）
- **触发条件**：`cards.json` **缺失或为 `[]`**，而 `campaigns.json` / `conversations/` 等仍在（例如手工删除、同步/云盘占位、S-06 的迁移复制失败）→ `card_ids` 为空 → 所有 Campaign 判"悬空卡孤儿"丢弃 → instances/knowledge/tasks/summaries/turns 全部按孤儿跳过 → 快照只剩 conversations → 校验自洽通过 → 写入 `SqliteAuthoritative` marker → **JSON 源树永不再读**（`cutover.rs:862-865` 注释明写 "Never re-open JSON source trees"）。
- **影响**：用户在**首次 SQLite 启动**（默认路径，无需崩溃或并发）后看到"所有战役/角色/记忆消失"；数据未物理删除但应用内不可恢复，且 `skipped_orphan_rows` 只进 `tracing::info!`，用户零提示。
- **修复建议**：区分"文件缺失"与"文件为空"；`cards.json` 缺失时不得执行悬空卡过滤，应 fail-closed 并给出可见提示；把非零 `skipped_orphan_rows` 写入 `import_runs` 并在 UI 暴露/要求确认。
- **域报告**：S-01（`02-storage-lifecycle.md`）。

---

## 4. P1 发现（44 条）

> 复核列：**[A]** = Lead 已回到源码/运行验证；**[B]** = 域内自核（有 file:line 与摘录）；**[C]** = 域内标注疑似。

### 4.1 域1 domain + 基础 infra（3 条）

| ID | 标题 | 位置 | 复核 |
| --- | --- | --- | --- |
| D-02 | 世界书 `enabled` 双写：`set_enabled(true)` 不回写 `extra.enabled`，导出→重导入后**用户启用被静默回滚** | `domain/src/world_info.rs:216-221,287,291,94-104` | **[A]** |
| D-03 | `infra-vector` 损坏恢复未接 `write_fence`/`storage_health`，备份拷贝失败被 `let _ =` 丢弃 → 可能以空集覆盖向量文件 | `infra-vector/src/lib.rs:86-88,404` | [B] |
| D-04 | `StoryTime` 触发用严格相等，而 `compile_turn_dossier` 传 `story_clock=""` → **writer 路径上时间伏笔永不注入** | `domain/src/story_task.rs:149-154,233-247` + `app-pipeline/src/turn_dossier.rs:284-285` | **[A]** |

### 4.2 域2 存储与 Turn 生命周期（6 条）

| ID | 标题 | 位置 | 复核 |
| --- | --- | --- | --- |
| S-02 | cutover 两个崩溃窗口（publish→marker、migrate→版本对账）→ marker 判 `Stale` 硬拒绝启动；库层 `recover_or_verify` 本可自愈却不可达（注释与行为相反）。*域内摘要主张 P0，Lead 按"崩溃触发 + 可用性"定级 P1* | `cutover.rs:1029-1032,324-330,664-676,885-891` + `storage_backend.rs:1952-1957` | [B] |
| S-03 | fresh-start 权威身份与 DB 内容无关（`(data_dir, 常量空库 hash)`）+ stale 白名单为自由文本子串 → **空库可顶替有库 / 陈旧 JSON 可重发布** | `cutover.rs:576-584,419-427,1444-1470,603-607,377-379` | [B] |
| S-04 | JSON accept 出现"副作用已全部落盘却返回 Err"，且进程内无恢复入口 → Turn 停 `Committing`、Campaign 卡死到重启 | `turn_lifecycle.rs:654-701,406-411` + `commands/turns.rs:703-724` | [B] |
| S-05 | JSON 三处无条件写回缺 `Committing`/终态守卫（regenerate/mark_stale/soft_delete）→ 可把 `Committed/Degraded` 回退 `DraftReady`；SQLite 同操作显式拒绝 | `turn_lifecycle.rs:154-163` + `backend_workflows.rs:278-280` + `turn_store.rs:243-252` | [B] |
| S-06 | 旧数据目录迁移复制失败只 warn/吞错（且早于 `resolve_backend`）→ 部分布局永久化，是 P0-3 的现实触发器 | `tauri-app/src/lib.rs:503-529` | [B] |
| S-07 | 启动恢复把活动 Turn 标 Failed 的**写盘失败被完全吞掉**（无日志）→ Campaign 无提示卡死 | `turn_lifecycle.rs:937-942` | [B] |

### 4.3 域3 写作流水线（6 条）

| ID | 标题 | 位置 | 复核 |
| --- | --- | --- | --- |
| W-01 | **身份归一两套语义**：`with_temporaries_for` 用 `trim + lowercase` 去重，而 `find_instance_by_id_or_name` 是**精确相等** → 大小写/空白变体让子 Agent 失去 instance 绑定，persona/知识/变量全丢、隔离退化到扁平角色查询 | `app-agent/runtime.rs:656-679` + `domain/campaign_runtime.rs:50-57,97-130` | **[A]** |
| W-02 | Plan 缺 `character_id` 被静默改写成 `"unknown"` → 建临时实例 → Accept 时 `UpsertInstance` **落库成幽灵角色** | `app-pipeline/src/lib.rs:3921-3925` + `turn_lifecycle.rs:104-108` | [B] |
| W-03 | 非 Campaign 会话整卷 reroll 传 `generation_mode=null` → 不被模式校验分支接收，落回 legacy 全流程重写（首写却是显式 continuation） | `app-pipeline/src/lib.rs:1672-1715` + `useMessageVariants.js:167` | **[A]**（机制） |
| W-04 | **质量门禁子串误判**：`check_meta_description` 用 `text.contains("让我来"/"好的，我"/"没问题，我")` 判 **Error** → 正常对白触发 1×Editor auto-fix 并默认拦截 Accept | `app-pipeline/src/quality_gate.rs:90-119` | **[A]** |
| W-05 | 后处理 DTO 必填字段无 default：**单条畸形 → 知识/变量/任务全丢 + 多打一次 LLM** | `app-agent/postprocess.rs:189-205,347-398,85-92` | [B] |
| W-06 | 归档批次/embed 失败只 warn 但**水位仍推进** → 远记忆段永久空洞且永不重试 | `app-memory/archiver.rs:151-226` + `commands/conversations.rs:145-163` | [B] |

### 4.4 域4 Tauri 命令层（3 条）

| ID | 标题 | 位置 | 复核 |
| --- | --- | --- | --- |
| T-01 | 契约测试扫描盲区：基线用 `\._invoke\(`（要求字面点号），漏掉 `shellDocUrl.js` 的裸 `_invoke(...)` → 实测 `uniqueInvokeCount=169`，3 个 `card_shell_*` 命令**不在前端集合内**，改名/移除不会让测试变红 | `scripts/architecture/backend-baseline.mjs:60` + `frontend/src/utils/shellDocUrl.js:64,83,105,120` | **[A]**（Lead 已跑脚本复现） |
| T-02 | Card Studio **8 处 `let _ = store.update(...)`** 丢弃落盘错误 → 命令返回 Ok 但状态未落盘（308/814 是直接 Ok 路径；710/718 把"未落盘"伪装成"JSON 解析失败"） | `card_studio_api.rs:308,361,486,574,579,710,718,814` | **[A]**（Lead 已逐行确认 8 处） |
| T-03 | `card_shell_fetch_url` 非 async + `reqwest::blocking` + 30s 超时 → 在 IPC handler 内联阻塞（框架依据已核：`Cargo.lock` 解析 `tauri-macros 2.6.3`，语义与域内引用一致） | `commands/card_shell.rs:147-163` + `card_shell_cache.rs:89,250-255` | **[A]**（版本前提已核实） |

### 4.5 域5 前端（14 条）

| ID | 标题 | 位置 | 复核 |
| --- | --- | --- | --- |
| F-01 | **中文输入法按回车确认候选词会直接提交**（`@keydown.enter.exact.prevent="submit"`，全 src 无 `isComposing`/`compositionend`） | `design/writing/ComposerBar.vue:73` | **[A]**（守卫缺失已核实；IME 实际行为未真机复现） |
| F-02 | **「采纳」失败被静默吞掉**：catch 只 `console.error`，兄弟路径都用 `alertDialog + errorText` → 点采纳后界面毫无变化 | `composables/useMessageVariants.js:282-303` | **[A]** |
| F-03 | 开场卡壳加载态是死状态（模板零读取 `cardShellLoading`） | `AppV2.vue:216,434,449` | [B] |
| F-04 | 日志面裸插值错误对象 → 用户看到 `[object Object]` | `AppV2.vue:186,558` | [B] |
| F-05 | `ui/DataList` 选中态契约恒为 false（消费不存在的 `activeItem`） | `components-v2/ui/DataList.vue:11-14,31,34` | [B] |
| F-06 | `ui/DataList` 的 `select` 事件生产不可达且无人监听 | `DataList.vue:32` + `CardLibrary.vue:105-108` | [B] |
| F-07 | 「可能完成」的任务在 UI 上被永久锁死（无确认入口，与后端要求矛盾） | `CampaignTasksTab.vue:176` ↔ `story_task.rs:232` | [B] |
| F-08 | 任务状态渲染成「可能完成 (NaN%)」——前端读 `status.likely_completed*100`，真实 DTO 是 `{confidence:0.5}`，且测试把错形状冻结 | `utils/taskStatus.js:3` + `tests/task-status.test.mjs:14` | [B] |
| F-09 | 「设为当前活动」失败零提示，且被外层误报成「创建失败/导入失败」 | `CampaignPanel.vue:190-205,182,431-442` | [B] |
| F-10 | 活动列表加载失败伪装成「还没有活动档」 | `CampaignPanel.vue:142-150` | [B] |
| F-11 | 角色卡库失败被 `.catch(() => {})` 吞掉，空态与"数据丢失"不可区分 | `CardLibrary.vue:32-42` | [B] |
| F-12 | 写卡工作室「保存产物」失败仍继续编译导入，编辑静默不生效 | `CardStudio.vue:454-474,547-559` | [B] |
| F-13 | 会话历史加载失败静默；单个 DTO 缺 `updated_at` 会清空整个历史列表 | `useConversation.js:74-86` | [B]（触发条件疑似） |
| F-41 | 提示词钩子审计面板「展开详情」100% 抛 `ReferenceError`（形参名错误），面板唯一可读详情永远打不开 | `PromptHookAuditLog.vue:37-43` | [B] |

### 4.6 域6 Meta/MVU/插件/Card Shell（7 条）

| ID | 标题 | 位置 | 复核 |
| --- | --- | --- | --- |
| M-02 | 卡壳网络白名单可绕过：`host_of` 手写解析与 `url`/reqwest 语义分歧 → `https://evil.example\@cdn.jsdelivr.net/x` 在 `host_of` 下是白名单主机、在 reqwest 下是 `evil.example`（SSRF + 逃逸 + 废掉 IP 字面量检查） | `tauri-app/src/card_shell_cache.rs:429-440` | **[A]** |
| M-03 | typed patch 未与 Campaign 绑定：`TypedPatch` 无 `campaign_id`，accept 只信前端入参 → A 战役提案可写进 B 战役；"foreign-campaign" 测试覆盖不到 | `app-meta/src/typed_patch.rs:72-86` + `commands/meta_typed.rs:280-292,189-201` | [B] |
| M-04 | MVU JS fallback 回写绕过 `__storyforge*` 保留命名空间守卫（守卫只在前端一条通道，全仓 `*.rs` 零命中）→ 可覆盖卡壳变量桶 | `MvuJsRuntime.vue:447` + `app-pipeline/src/lib.rs:1534-1541` | [B] |
| M-05 | 世界书 `position` 导出为数字，ST V2/V3 规格为字符串枚举 → 对外互操作契约破 | `domain/src/world_info.rs:286` | [B]（Lead 已确认代码写数字；规格口径未独立核对） |
| M-06 | 保真闸门失灵：3 个真实卡测试全部 `#[ignore]` 且默认 fixture 路径不存在；6 个 fixture 均为 135B~60KB 手工桩；`compat.rs` 把 position 漂移硬编码为 `Intentional` | `infra-import/src/compat.rs:690,828,843` + `tests/*` | [B] |
| M-07 | `TavernHelperRuntime` 桥只认 `__sf_th_bridge` 标记、**不校验 `event.source`** → 任意 frame 可伪造 `register_module/prepare_remote_script/var_write`（区别于 `mvu-runtime-bridge.js` 的正确基线） | `TavernHelperRuntime.vue:680-683` | [B] |
| M-08 | `plugin_prompt_hook_result` 无 `plugin_id`、无 `ensure_permission`、未知 id 也返回 Ok → `ModifyPrompt` 唯一门禁在前端 JS | `commands/plugins.rs:243-254` + `commands/writing.rs:577-595` | [B] |

### 4.7 域7 目标与声明（5 条）

| ID | 标题 | 位置 | 复核 |
| --- | --- | --- | --- |
| G-01 | `RELEASE-STATUS.md:34` 自相矛盾：写「HEAD 为 ce6117d」且「工作区另有未提交的 SHA256SUMS 修复」，实际 HEAD=`ab894c6` 已含该修复、工作树干净（tag v0.1.2 才 = ce6117d）→ 门禁表第 1 行「源码和文档一致性」不成立 | `docs/RELEASE-STATUS.md:34,30,84` | **[A]**（Lead 前期已独立发现） |
| G-02 | README 承诺的平台命名校验和在 v0.1.2 发布物中不存在（实际只有 `SHA256SUMS.txt` + `SHA256SUMS-android.txt`） | `README.md:43` | [B]（域7 经 GitHub API 核实） |
| G-03 | README 的 APK 通配 `*-arm64-*-release.apk` 与实际资产名 `app-arm64-release.apk` 不匹配，用户按文档找不到 | `README.md:41` + `.github/workflows/release.yml:247` | [B] |
| G-04 | README:25 仍称"真实模型移动写作/第三方插件/正式分发安装包/远端 CI 仍须闭合"，与 RELEASE-STATUS 四项"已闭合"直接冲突 | `README.md:25` | [B] |
| G-05 | ARCHITECTURE/HANDOFF 仍把 Windows runner/Android 真机/release 签名/第三方插件列为缺证据，与 RELEASE-STATUS 的"已闭合"并存（未区分"Gitea 自托管 runner 已停用"这一真实残余） | `ARCHITECTURE.md:174,180-181,187` + `HANDOFF.md:16,30-31,79,137` | [B] |

---

## 5. 跨域主题分析

### 5.1 头号系统性缺陷：静默失败（贯穿 6/7 个域）

按题意统计，属于"失败被吞掉/降级为成功/伪装成空态"的发现至少有 **24 条**：S-06/S-07/S-13/S-14（`let _ =` 与 warn）、T-02（8 处落盘错误）、F-02/F-09/F-10/F-11/F-12/F-13/F-45（前端吞错与空态假象）、W-05/W-06（部分失败全丢/水位推进）、D-03（备份失败丢弃）、M-06（闸门把漂移标为 Intentional）、S-01（P0，静默丢数据）。

这不是零散 bug，而是**缺少统一的失败传播约定**：同一项目里既有"失败必须传播"的严格实现（SQLite UoW、命令错误 DTO），也有大量 `let _ =`、`.catch(() => {})`、`console.error` 的局部妥协。建议立一条项目级规约并在 CI 中做静态检查（例如禁止新增 `let _ = store.update(...)` 形态）。

### 5.2 护栏"宣称强于实际"

| 宣称 | 实际 | 证据 |
| --- | --- | --- |
| 契约测试保证"每个前端 invoke 都在后端注册" | 对 3 个 live 调用点**永久失明**，且该测试不入 CI | T-01 / T-10 |
| Card Studio 状态可靠落盘 | 8 处落盘错误被丢弃 | T-02 |
| ST 导入导出保真 | 真实卡测试全 `#[ignore]`，fixture 为手工桩，漂移被标为 `Intentional` | M-06 |
| Meta patch "不越权改数据" | preview 只是 UI 约定；typed patch 不与 Campaign 绑定；revision 陈旧性校验对 LLM 提案恒被跳过 | M-03 / 域6 §2 |
| 卡壳 CSP fail-closed | 子帧 IPC 通道可回退，三道门可同时失效 | P0-2 |
| 插件权限桥强制最小权限 | 前端承担 `__storyforge*` 命名空间守卫；prompt hook 结果无 plugin_id 校验 | M-04 / M-08 |

### 5.3 双路径语义分叉

项目同时维护 **SQLite（默认）** 与 **JSON（显式回退）** 两套提交语义，审查确认：**SQLite 侧严格实现，JSON 侧系统性偏弱**（accept 缺 `Committing`/终态守卫、失败语义为"已提交却报错且不可自愈"、多处吞错）。另有 Campaign/legacy 双路径的身份语义分叉（W-01）与前端/后端守卫分叉（M-04/M-08/T-06）。建议明确 JSON 路径的定位（限期回退即可，但**必须与 SQLite 同语义或显式拒绝高风险操作**）。

### 5.4 冗余与死代码清单（可执行）

| 类别 | 规模 | 证据 |
| --- | --- | --- |
| 零引用/仅测试引用的前端组件 | **18**（`components-v2/writing/**` 8 个整树 + `ui/` 9 个 + `BaseDropdown` 经死代码传递；域6 另交叉 1 个） | F-18/F-30/F-46 |
| 孤儿前端 wrapper | **21**（`tauri-api.js` 导出 160，`src/**` 零命中） | F-30 |
| 孤儿后端命令 | **3**（`abandon_turn`/`archive_conversation`/`soft_delete_variant`，wrapper 已删但命令与注册残留） | T-15 |
| 死代码模块 | `app-agent/src/tool_center.rs`（且内置"幻影工具名"）；`components-v2/writing/**` | W-22 / F-40 |
| 重复实现 | `format_subagent_context_*` 在 pipeline 与 app-agent 各一份；生产页手写 ui 基元同款 | W-32 / F-18 |
| 文档指向死代码 | `FRONTEND-COMPONENTS.md` 仍把 `components-v2/writing/**` 当写作面 | F-40 |
| 代码债务标记 | Rust `TODO/FIXME/HACK/XXX` 28 处（tauri-app 19、domain 5）；前端 5 处；`#[allow(dead_code)]` 6 处 | Lead 统计 |
| 未实现桩 | `StubMvuRuntime`（文档已声明为降级路径，不计缺陷）；无 `todo!`/`unimplemented!` | Lead 统计 |

### 5.5 前端可用性专项（用户重点关注项）

**结论：主流程可用、无 P0、无断链**（16 个写作事件与 11 个 campaign 事件都有监听者；`WritingScreen` 历史 payload 丢失 bug 已修）。问题集中在三类：

1. **失败不可见**（F-02/F-09/F-10/F-11/F-12/F-13/F-45）：命令失败后界面显示"空列表"或"无反应"，用户会误判数据丢失或以为已保存。
2. **契约/形状漂移**（F-05/F-06/F-08/F-42/F-43/F-44）：`activeKey` 恒 false、`likely_completed` 渲染 `NaN%`、`latencyMs` vs `latency_ms`、角色列表缺 `Writer`、trace 面板不认识 `writer_*`。
3. **交互缺陷**（F-01 IME 误提交、F-07 任务永久锁死、F-41 审计面板必抛错、F-24 写操作无 busy 防重）。

**组件测试覆盖**：41/96（上界口径），7 个 campaign 面板、4 个 config 面板无任何测试引用；`npm test` 不含 vitest（修 P1 前须 `npm test && npm run test:ui`）。

### 5.6 安全专项

| 级别 | 项 | 状态 |
| --- | --- | --- |
| P0 | M-01 子帧 IPC/ACL 暴露 | 静态 4/5 环已核实；**运行时 PoC 未跑** |
| P1 | M-02 白名单/SSRF 绕过（`host_of`） | **[A]** 已核实 |
| P1 | M-03 typed patch 跨 Campaign 写入 | [B] |
| P1 | M-04 变量命名空间守卫仅前端 | [B] |
| P1 | M-07 postMessage 桥不校验 `event.source` | [B] |
| P1 | M-08 prompt hook 无权限校验 | [B] |
| 正向 | capability 最小权限用法正确；插件 bridge 固定 allowlist 无法构造任意命令名；错误 DTO 不泄露密钥；`secret_store` 未发现明文残留 | 域4/域1 |

---

## 6. 各域"未发现问题"与正面结论

> 完整清单见各分域报告 §5。此处摘录，防止报告只呈现缺陷面。

- **命令层与注册**：175 ↔ 175 双向一致；165 个 JS 调用站点仅 1 处参数漂移；0 处命名风格误用；生产路径 `unwrap` 全 src 仅 20 处（commands 仅 1 处不变量断言）；无持锁跨 await；129 个生产 DTO 仅 1 组同名且属不同层。
- **SQLite 提交链路**：Accept UoW 原子性、事务内 CAS、ledger 幂等、late guard、Degraded 不可逆、失败传播——逐行核对成立，未发现"响应成功而磁盘滞后"或"重复 accept 二次扣减"。
- **domain 纯度与事实一致性**：无 tauri/网络/IO 依赖；CLAUDE.md/DATA_MODEL.md 关于 `CharacterInstance`、`CharacterDefinition`、`resolved_*` fallback、`CampaignRuntimeContext`、`AgentProfileConfig`、Chronicle A/B/C、Turn/Attempt 的条目**逐条与代码一致**；`normalize_mvu_key` 与前端 `mvuKey.js` 镜像语义等价。
- **写作流水线**：工具循环对畸形 JSON/取消/max rounds 的处理、Semaphore 并发与 index 对齐、子 Agent 绑定后硬失败防泄漏、顺序剧组公开拍隔离、UTF-8 安全截断、`pending_temporary_instances` 清理、whitelist 四种分支、`enable_*` 门禁——均未发现问题。
- **数据面**：PNG tEXt/CRC/UTF-8、`flatten extra` 未知字段保留、Bundle 外键与版本门、SQLite 单事务回滚、MVU apply 幂等与不覆盖存量、`value_expr` 无任意 JS 执行面——逐条核实无问题（域6 §5.1 共 20 项）。
- **门禁与数量事实**：16 crate、175 命令、1980/0/33、504+119、198 项 Pester、三轮门禁日志一致；未完成项均如实标注；范围内文档相对链接无失效。

---

## 7. 修复优先级建议

### 批次 1：立即（P0 + 高收益低风险 P1）

1. **P0-1** 卡壳 UTF-8 panic：1 行守卫 + 中文回归测试。
2. **P0-3** 迁移 fail-closed：`cards.json` 缺失/为空不再执行悬空卡过滤；`skipped_orphan_rows` 可见化。
3. **P0-2** 先跑 30 秒运行时 PoC；成立则按 3.2 建议修（ACL manifest + CSP `ipc:` + `event.source` 校验）。
4. **W-01** 名字归一统一（id/name 匹配改为 `trim + 大小写不敏感`，或统一为规范化后比较）。
5. **T-01 + T-02**：修基线正则并补"后端命令是否存在前端零引用"的反向断言；Card Studio 8 处改为 `?` 传播。
6. **F-01 + F-02**：IME 组合态守卫；采纳失败 `alertDialog(errorText(e))`。

### 批次 2：本轮内（P1 收敛）

S-02/S-03（cutover 自愈入口与身份 nonce）、S-04/S-05（JSON 路径守卫）、S-06/S-07（吞错改传播）、W-02/W-03/W-04/W-05/W-06、T-03（async + `spawn_blocking`）、D-02/D-04、M-02..M-08、F-03..F-13/F-41、G-01..G-05（文档回写）。

### 批次 3：稳定期（债务与治理）

- 死代码清理：18 组件 / 21 wrapper / 3 命令 / `tool_center.rs`；同步修文档指向（F-40/F-46）。
- 把契约测试、capability 测试、前端快照纳入 CI（T-10）；补 `npm test && npm run test:ui` 的合并入口。
- 明确 JSON 回退路径语义（与 SQLite 齐平或对高风险操作显式拒绝）。
- 真实卡保真测试改为 CI 可跑的夹具（M-06），去掉 `Intentional` 硬编码豁免。
- 立"禁止新增吞错"的静态检查（`let _ = store.update`、`.catch(() => {})`）。

---

## 8. 未验证 / 不可验证项（诚实边界）

1. **P0-2 运行时 PoC 未执行**（最高优先待办）：需装 `permissions:[]` 插件观察 `window.__TAURI_INTERNALS__`/`window.ipc`。
2. **真实模型行为**：所有 `#[ignore]` 真实 LLM 用例本次未运行；涉及"LLM 行为层隔离/传播"的结论均为静态推导。
3. **GUI/真机/构建产物**：Playwright 三套未跑；Windows/Android 真机、APK 签名（`apksigner`）、Gitea 服务端停用状态无法本地验证。
4. **运行时时序类疑似项**：S-03 多进程租约窗口、S-08 WAL checkpoint、S-10 WAL 静默退化、S-22.7/S-22.8、F-13 触发条件、M-19 内联壳信任粒度。
5. **未独立核对的外部规格**：M-05 的 ST position 类型（代码事实已确认是数字，规格口径未复核）、M-01 第 6 环。
6. **域7 的 G-12**：校验和"android 覆盖 windows"方向为**中高置信疑似**，未取回发布物正文核实。

---

## 9. 附录

### 9.1 分域报告索引

| 文件 | 内容 |
| --- | --- |
| `01-domain-infra.md` | domain + infra-util/vector/regex：D-01..D-26 |
| `02-storage-lifecycle.md` | infra-sqlite + 存储/Turn 生命周期：S-01..S-22 |
| `03-writing-pipeline.md` | app-agent/pipeline/conversation/memory：W-01..W-32 |
| `04-tauri-commands.md` | 175 命令逐条契约表（§4.1，175 行）：T-01..T-16 |
| `05-frontend.md` | 前端通用层 + 组件可用性核对表：F-01..F-47 |
| `06-meta-mvu-plugin.md` | Meta/MVU/插件/Card Shell/导入导出：M-01..M-32 |
| `07-goals-and-claims.md` | 39 项目标逐条核对表 + 数量类事实核对表：G-01..G-19 |

### 9.2 本次审查产生的证据文件

- `artifacts/review-2026-09-13/fmt.log`、`clippy.log`、`cargo-test.log`、`fe-node-test.log`、`fe-vitest.log`、`fe-build.log`
- 环境说明：前端三套命令首次在受限沙箱下因 Node/esbuild `spawn EPERM` 失败，经一次沙箱放宽后取得真实结果（与项目代码无关）。

### 9.3 复核命令清单（可自行重跑）

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cd frontend; npm.cmd test; npm.cmd run test:ui; npm.cmd run build
node scripts/architecture/backend-baseline.mjs     # 复现 T-01：uniqueInvokeCount = 169
git show --stat ab894c6                            # 复现 G-01
```

### 9.4 分域报告内部计数不一致（Lead 裁决记录）

| 报告 | 不一致 | Lead 裁决 |
| --- | --- | --- |
| `02-storage-lifecycle.md` | §2 写「P0 2 条、P1 5 条」，而发现标题为 S-01 = P0、S-02..S-07 = P1（即 P0 1 / P1 6） | 采信**发现标题**：域2 = P0 1 / P1 6 / P2 11 / P3 4（22 条）。S-02 按"崩溃触发 + 可用性"定级 **P1**，若产品认为"崩溃后永久无法启动"不可接受可上调 P0 |
| `05-frontend.md` | §2 曾写「P1 13 / P2 17 / P3 10（40 条）」，最终文件已更新为 **P1 14 / P2 21 / P3 12（47 条）** | 采信最终值（本报告已按 47 条计） |
| 任务书 vs 仓库 | 域2 任务书写 `V001..V007`，仓库实际 `V001..V008`（schema v8） | 以代码为准；不影响结论 |

---

**报告结束**。本报告的所有 P0 与 14 条 P1 已由 Lead 回到源码（必要时运行隔离复现或脚本）验证；其余 P1/P2/P3 的证据出处见对应分域报告的 file:line 与代码摘录。审查过程全程只读，未改动任何源码、测试或配置。

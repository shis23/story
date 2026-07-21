# StoryForge 交接说明

> 更新日期：2026-07-21
> 代码事实基线：`bf36e04`；自动化验证基线仍以最近一次完整 `verify-release` 记录为准（历史记录见 2026-07-14 段）。
> 范围：当前代码事实、证据等级、验证入口与下一优先级。
> SQLite 专线审计：`docs/workstreams/SQLITE-CURRENT-STATUS-AUDIT-2026-07-21.md`。

## 当前结论

StoryForge 已进入工程化发布候选阶段。Campaign-first 多 Agent 写作、Turn/Attempt 一致性、Phase B 质量与私密知识门禁、Chronicle M0–M4.2.2、插件/导入兼容硬化、opt-in SQLite（含 pre-accept 生产接线）和发布证据脚本均已形成可执行主线。

当前不能宣称正式发布或完整 M5 / SQLite 真实证据验收，主要缺口是：

1. 旧 JSON 路径 M5 Full 真实模型证据仅 45/100 Accept；原始 JSONL 已清理，不能续跑。
2. SQLite 生产接线（opt-in）已覆盖 draft / autofix / postprocess / regenerate / edit-stale / Accept / recovery，但 **没有** 已提交的 `SQLITE-M5-100-ENDURANCE-RESULT.md`；Native 12 曾 accepted 12/12，seal 因文件锁失败，不得写成 PASS。
3. 旧 JSON M5 harness 与当前 SQLite endurance harness 必须分开记账：前者不覆盖 SQLite Accept；后者走 `sqlite_runtime` / `ProductionPostprocessService` / `accept_by_variant`，仍非完整 GUI command 端到端，且 Meta typed patch 等在 SQLite 下显式 unsupported。
4. Gitea runner、桌面真实 GUI、Android 真机、签名安装包和第三方插件 iframe 仍缺现场证据。
5. CoT 三臂 × 80 轮真实验证计划在库中，RESULT 未写出；与 SQLite 接线正交。

## 已落地主线

| 领域 | 当前状态 |
| --- | --- |
| Campaign 写作 | Director → Subagent → Editor 主路径已接；ScenePlan、NarrativeContract、agency 字段进入提示词与 Gate |
| Turn 一致性 | TurnRecord / TurnAttempt、draft hash、revision CAS、MutationBatch、Accept 屏障、迟到 postprocess 守卫、崩溃恢复已接 |
| Phase B / B2 | 私密归属契约、显式探针、文本窗口 attribution、Editor performance redaction、1× Editor auto-fix 已接 |
| 记忆 / Context | ContextEpoch、near_raw、A/B/C 查询工具、ChronicleCompressor job/publication、cache usage 与 segment 观测已接 |
| SQLite | 默认仍为 JSON；opt-in 后 cutover/marker、pre-accept UoW、Accept UoW、recovery、barrier、备份和 reverse export 已接生产路径；真实证据未封存 |
| 插件 | prompt hook、权限撤销、预算/超时/取消、审计链、兼容矩阵与显式 degraded/unsupported 行为已接 |
| 导入/导出 | ST/Campaign Bundle 兼容矩阵、原子失败、引用校验、fixture corpus 和脱敏报告已接 |
| 发布证据 | Windows/Android host runner、manifest/provenance/hash、Gitea workflow 已提交；远端 runner 实跑尚未验证 |

## LLM Request Policy

- 主写作默认 `max_tokens=None`，请求体省略该字段，由 endpoint/model 决定默认输出上限。
- 历史未标记的 `4096` 视为旧 UI 默认，不会突然变成生产硬上限。
- 用户显式填写正整数时才发送上限；`4096`、`384000` 等均会按用户意图透传。
- M5 可用 `STORYFORGE_EVAL_MAX_TOKENS` 覆盖评估请求；该值现在作用于实际 `ChatRequest`，不是连接对象上的无效字段。
- 专用请求仍可有独立上限，例如连接 ping、JSON fallback 和 MemoryArchiver；不得把它们描述成主写作限制。

## M5 / Phase B 证据

### 旧 JSON 路径（历史 partial）

权威结果：`docs/workstreams/M5-PHASEB-100TURN-EVIDENCE-RESULT.md`。

| 阶段 | Accept | Calls | Epoch | 结论 |
| --- | ---: | ---: | ---: | --- |
| Canary | 3/3 | 30/30 | 1 | Pass |
| Coverage | 12/12 | 106/120 | 1 | Pass |
| Stability | 30/30 | 282/300 | 3 | Pass |
| Full | 45/100 | 415/700 | 5 | Partial Evidence |

证据边界：

- 写作入口：production pipeline。
- Accept：probe 调用共享 JSON `TurnLifecycleService`；**不**覆盖 Tauri SQLite Accept 路径。
- Chronicle：记录中含 `synthetic_chronicle_fixture`，不得写成完整生产 postprocess 验收。
- 原始 45/100 证据曾位于外部目录且已清理；只能作为历史证据，不能续跑。
- 不得据此修改或宣称已标定 `200/4`、`H_anchor=5`、`E=10`。

### SQLite 路径（代码已接，证据未封存）

当前事实入口：`docs/workstreams/SQLITE-CURRENT-STATUS-AUDIT-2026-07-21.md`。  
计划：`docs/workstreams/SQLITE-M5-100-ENDURANCE-PLAN.md`。  
RESULT：`docs/workstreams/SQLITE-M5-100-ENDURANCE-RESULT.md`（**Partial Evidence**，非封存 PASS）。

- harness：`crates/harness-real-llm/src/sqlite_endurance.rs` 经 `sqlite_runtime` + `ProductionPostprocessService` + `accept_by_variant`。
- 生产命令：`start_writing` / autofix / postprocess / regenerate / edit-stale 在 opt-in SQLite 下走 preaccept UoW。
- 本地旁证：Native 12 曾 `turns=12/12 accepted`，seal 因 live SQLite 文件锁失败；2026-07-21 已修 seal 扫描，**未**重跑真实模型封存，**不得**写成 PASS。
- Meta：campaign health 可读 SQLite；typed patch 等仍显式 unsupported。

## 存储边界

默认行为保持 JSON，避免在未完成证据与真机验收前强制切换用户数据。

SQLite opt-in **已覆盖**（代码接线，默认仍 JSON）：

- 类型化 backend selector 与进程 pin。
- fail-closed cutover、marker、锁、内容 hash 重算和启动恢复。
- pre-accept：draft / autofix / postprocess / regenerate / edit-stale / outbox / recovery（`SqlitePreacceptRepository` + `sqlite_runtime` + Tauri 命令分支）。
- Turn Accept / recovery / active-turn barrier 权威路径。
- Chronicle publication UoW、故障注入回滚、备份和 SQLite→JSON reverse export。
- 确定性生产网关测试：`crates/tauri-app/tests/sqlite_preaccept_production_lifecycle.rs`。

仍需补齐：

- 提交 `SQLITE-M5-100-ENDURANCE-RESULT.md`（含 seal / offline verify / secret scan；诚实 PASS 或 partial）。
- Meta typed patch / MVU 等 SQLite-native UoW（当前 fail-closed unsupported）。
- Windows/Android 真正启用 SQLite 后的文件锁、生命周期和大数据现场验证。

## 2026-07-14 验证记录

- `cargo fmt --all -- --check`：通过。
- `cargo clippy --workspace --all-targets -- -D warnings`：通过。
- `cargo test --workspace`：通过；真实模型、OS credential store 等用例按设计 ignored。
- `cargo test -p harness-real-llm`：全部确定性 suite 通过；真实模型用例按设计 ignored。
- `frontend npm.cmd test`：311/311 通过。
- `frontend npm.cmd run build`：通过；保留既有 Vite dynamic/static import warning。
- 本轮未新增真实/付费模型调用。

## 验证入口

完整确定性发布闸门：

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\verify-release.ps1
```

真实模型 smoke 只从环境变量读取凭证：

```powershell
$env:LLM_BASE_URL='https://your-compatible-endpoint/v1'
$env:LLM_API_KEY='<secret-from-shell>'
$env:LLM_MODEL='<model>'
powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\run-real-llm-smoke.ps1 -Suite knowledge
```

M5 endurance 默认不发送输出上限；如需显式 ceiling：

```powershell
$env:STORYFORGE_EVAL_MAX_TOKENS='384000'
```

## 下一优先级

1. 用已修复的 seal 路径重跑 SQLite Coverage Native 12（新 run id）并 seal+verify；再做 TextFallback 专项；不复活旧 45/100。
2. 仅在 RESULT 需要时补 SQLite Meta UoW / 真机 opt-in 现场；未支持能力保持显式 unsupported。
3. CoT 三臂 × 80 轮：先核 Gate 0 确定性门，再跑真实三臂并写 RESULT（与 SQLite 封存分账）。
4. 部署并实跑 Gitea runner，验证 workflow、上传包、subject/sidecar 和离线 re-hash。
5. 由人工补桌面 GUI、Android 真机、签名包和真实第三方插件验收。

## 交接约束

- 不把真实 API key 写入仓库、文档、日志、截图或证据 JSONL。
- 不把模型最大能力窗口等同于每次请求应设置的输出长度；`max_tokens` 是 ceiling，不是目标长度。
- 不把 deterministic fixture、可执行入口或 synthetic Chronicle 写成完整生产验收。
- 不默认切换 SQLite；必须保留 fail-closed cutover、备份和 reverse export。
- `docs/archive/**` 与旧 workstream PLAN/RESULT 是历史证据，除非修正事实错误，否则不回写成当前状态。

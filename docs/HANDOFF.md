# StoryForge 交接说明

> 更新日期：2026-07-15
> 代码验证基线：`99b1ea3`；文档同步后请以当前 `main` 为准。
> 范围：当前代码事实、证据等级、验证入口与下一优先级。

## 当前结论

StoryForge 已进入工程化发布候选阶段。Campaign-first 多 Agent 写作、Turn/Attempt 一致性、Phase B 质量与私密知识门禁、Chronicle M0–M4.2.2、插件/导入兼容硬化、opt-in SQLite 和发布证据脚本均已形成可执行主线。

当前不能宣称正式发布或完整 M5 验收，主要缺口是：

1. M5 Full 真实模型证据仅 45/100 Accept。
2. M5 harness 复用 production Pipeline，且通过 probe 调用共享 JSON `TurnLifecycleService`；它不执行 Tauri command 或 SQLite Accept 路径。Summarizer/PostProcessor/TurnAttempt 后台写回与 Chronicle A 仍未形成可供 harness 复用的完整生产应用服务。
3. SQLite opt-in 已成为 Accept/recovery/barrier 的权威路径，但部分 pre-accept draft/postprocess 命令仍需继续迁移，避免混合后端生命周期。
4. Gitea runner、桌面真实 GUI、Android 真机、签名安装包和第三方插件 iframe 仍缺现场证据。

## 已落地主线

| 领域 | 当前状态 |
| --- | --- |
| Campaign 写作 | Director → Subagent → Editor 主路径已接；ScenePlan、NarrativeContract、agency 字段进入提示词与 Gate |
| Turn 一致性 | TurnRecord / TurnAttempt、draft hash、revision CAS、MutationBatch、Accept 屏障、迟到 postprocess 守卫、崩溃恢复已接 |
| Phase B / B2 | 私密归属契约、显式探针、文本窗口 attribution、Editor performance redaction、1× Editor auto-fix 已接 |
| 记忆 / Context | ContextEpoch、near_raw、A/B/C 查询工具、ChronicleCompressor job/publication、cache usage 与 segment 观测已接 |
| SQLite | 默认仍为 JSON；显式 opt-in cutover、marker、Accept UoW、recovery、barrier、备份和 reverse export 已接 |
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

权威结果：`docs/workstreams/M5-PHASEB-100TURN-EVIDENCE-RESULT.md`。

| 阶段 | Accept | Calls | Epoch | 结论 |
| --- | ---: | ---: | ---: | --- |
| Canary | 3/3 | 30/30 | 1 | Pass |
| Coverage | 12/12 | 106/120 | 1 | Pass |
| Stability | 30/30 | 282/300 | 3 | Pass |
| Full | 45/100 | 415/700 | 5 | Partial Evidence |

证据边界：

- 写作入口：production pipeline。
- Accept：probe 调用共享 JSON `TurnLifecycleService`；不覆盖 Tauri command 与 SQLite Accept 路径。
- Chronicle：当前仍含 `synthetic_chronicle_fixture`，`production_postprocess_complete=false`。
- 原始 45/100 证据曾位于 `C:\tmp\endurance-evidence-full`、不在 Git 追踪范围；当前该目录已随清理移除，因此结果只能作为已记录的历史证据，不能从 checkpoint 续跑。
- 不得据此修改或宣称已标定 `200/4`、`H_anchor=5`、`E=10`。

## 存储边界

默认行为保持 JSON，避免在未完成全生命周期迁移前强制切换用户数据。

SQLite opt-in 已覆盖：

- 类型化 backend selector 与进程 pin。
- fail-closed cutover、marker、锁、内容 hash 重算和启动恢复。
- Turn Accept / recovery / active-turn barrier 权威路径。
- Chronicle publication UoW、故障注入回滚、备份和 SQLite→JSON reverse export。

仍需补齐：

- `append_ai_draft`、Attempt 中间态、autofix/postprocess 写回等完整 pre-accept 生命周期。
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

1. 抽出共享 ProductionPostprocessService：让 Tauri 与 harness 复用 Summarizer、PostProcessor、Attempt 同步、Chronicle A 发布和迟到结果守卫。
2. 在完整生产 postprocess 路径接通后运行新的 M5 100-Accept 证据；现有 45/100 仅作为已记录的旧路径基线保留，因原始 JSONL 已清理不能继续该 checkpoint。
3. 完成 SQLite pre-accept draft/postprocess 全生命周期迁移与故障注入。
4. 部署并实跑 Gitea runner，验证 workflow、上传包、subject/sidecar 和离线 re-hash。
5. 由人工补桌面 GUI、Android 真机、签名包和真实第三方插件验收。

## 交接约束

- 不把真实 API key 写入仓库、文档、日志、截图或证据 JSONL。
- 不把模型最大能力窗口等同于每次请求应设置的输出长度；`max_tokens` 是 ceiling，不是目标长度。
- 不把 deterministic fixture、可执行入口或 synthetic Chronicle 写成完整生产验收。
- 不默认切换 SQLite；必须保留 fail-closed cutover、备份和 reverse export。
- `docs/archive/**` 与旧 workstream PLAN/RESULT 是历史证据，除非修正事实错误，否则不回写成当前状态。

# StoryForge 交接说明

> 更新日期：2026-08-05
> 代码事实基线：`main@065629e`（Gate 7/8 完成后）；自动化验证基线以最近一次完整 `verify-release` 记录为准（Release gate passed，2026-08-05）。
> 范围：当前代码事实、证据等级、验证入口与下一优先级。
> 本专项（后端拆分 + SQLite 收口）的权威结果：`docs/workstreams/BACKEND-ARCHITECTURE-SQLITE-CLOSURE-RESULT-2026-07-28.md`。

## 当前结论

StoryForge 处于发布候选阶段。Campaign-first 多 Agent 写作、Turn/Attempt 一致性、
质量与私密知识门禁、Chronicle M0–M4.2.2、插件/导入兼容硬化、SQLite 生产路径与
发布证据脚本均已形成可执行主线。**SQLite 已切换为默认后端（Gate 7，2026-08-05）**，
JSON 保留为显式回退。

当前不能宣称正式发布或 Gate 6 完整封存，主要缺口是：

1. Gate 6 真实模型 Full100 证据受 relay 间歇不稳定阻断（r3 跑到 58/100 全健康，
   见 RESULT §35.8），续跑中；Canary3 / Coverage12 / TextFallback3 / Stability30
   已 PASS 并 seal。
2. Gate 7 的「完整候选周期」统计（自动回退率/迁移失败率，§12.1.2）与删除 JSON
   生产写路径（§12.1.5）留待发布后/稳定期后。
3. Gitea runner：Linux 已投入运行；Windows runner、桌面真实 GUI、Android 真机、
   签名安装包和第三方插件 iframe 仍缺现场证据（release APK 签名无证书 BLOCKED）。
4. CoT 三臂 × 80 轮真实验证计划在库中，RESULT 未写出；与 SQLite 收口正交，按
   PLAN §16 单独排期。

## 已落地主线

| 领域 | 当前状态 |
| --- | --- |
| Campaign 写作 | Director → Subagent → Editor 主路径已接；ScenePlan、NarrativeContract、agency 字段进入提示词与 Gate |
| Turn 一致性 | TurnRecord / TurnAttempt、draft hash、revision CAS、MutationBatch、Accept 屏障、迟到 postprocess 守卫、崩溃恢复已接；启动恢复带 recovery_retries 上限（H-3，2026-08-04 review） |
| 质量/私密 | 私密归属契约、显式探针、文本窗口 attribution、Editor performance redaction、1× Editor auto-fix 已接 |
| 记忆 / Context | ContextEpoch、near_raw、A/B/C 查询工具、ChronicleCompressor job/publication、cache usage 与 segment 观测已接 |
| SQLite | **默认后端（Gate 7）**：无配置启动即 SQLite；旧 JSON 自动迁移（缺失集合=空、孤儿行跳过并计数、备份后发布、不删旧 JSON）；全新目录初始化空库；JSON 为显式回退（`STORYFORGE_STORAGE_BACKEND=json` / JsonAuthoritative marker） |
| 插件 | prompt hook、权限撤销、预算/超时/取消、审计链、兼容矩阵与显式 degraded/unsupported 行为已接 |
| 导入/导出 | ST/Campaign Bundle 兼容矩阵、原子失败、引用校验、fixture corpus 和脱敏报告已接 |
| 发布证据 | Windows host runner、manifest/provenance/hash、Linux Gitea runner 已投入运行；Windows runner 实跑尚未验证 |

## LLM Request Policy

- 主写作默认 `max_tokens=None`，请求体省略该字段，由 endpoint/model 决定默认输出上限。
- 历史未标记的 `4096` 视为旧 UI 默认，不会突然变成生产硬上限。
- 用户显式填写正整数时才发送上限；`4096`、`384000` 等均会按用户意图透传。
- M5 可用 `STORYFORGE_EVAL_MAX_TOKENS` 覆盖评估请求；该值现在作用于实际 `ChatRequest`，不是连接对象上的无效字段。
- 专用请求仍可有独立上限，例如连接 ping、JSON fallback 和 MemoryArchiver；不得把它们描述成主写作限制。

## 真实模型证据（Gate 6）

权威结果：`docs/workstreams/BACKEND-ARCHITECTURE-SQLITE-CLOSURE-RESULT-2026-07-28.md` §35。

| 阶段 | 结果 | 备注 |
| --- | --- | --- |
| Canary 3 | PASS（seal） | run_id 见 RESULT §35.2.1 |
| Coverage 12（Native） | PASS（seal） | §35.2.2 |
| TextFallback 3 | PASS（seal） | §35.2.3 |
| Stability 30 | PASS（seal） | §35.2.4 |
| Full 100 | **BLOCKED**（4 次重跑受 relay 间歇不稳定阻断） | r3 跑到 turn 58/100 全健康（564 次真实调用），深度分析 §35.8；harness 侧 15h ceiling + 8-attempt 重试已修，待 relay 恢复后 `run-stage.sh full native 100 3500` 续跑 |

证据边界：

- 旧 JSON 路径的 45/100 是历史 Partial Evidence（原始 JSONL 已清理），**不复活、不视为 PASS**。
- 不得据此宣称已标定 `200/4`、`H_anchor=5`、`E=10`。
- API key 只从环境变量读取，不写入仓库/文档/日志/证据。

## 平台现场证据（Gate 6 §11.3 + Gate 7）

- **Android 模拟器**（§11.3）：15 项验收点全部现场 PASS（APK 安装、数据路径、
  JSON→SQLite 升级、keyring 迁移、content:// 导入、SAF 导出、生命周期、断网/取消、
  大数据+恢复）；发现并修复 2 个真实 Keystore 缺陷（#7 ndk-context panic、
  #8 迁移永不重试）。release APK 签名无证书 BLOCKED。
- **Windows 桌面**（§11.3）：启动、fail-closed、cutover、大数据、备份/reverse
  export、重启恢复、进程锁、schema 升级全 PASS（8 个确定性套件 + 现场启动）。
- **Gate 7 默认切换现场**（2026-08-05，`storyforge-evidence/gate7-2026-08-05/field/`）：
  全新用户默认启动（空 APPDATA + 无 env → SQLite 零 JSON）、真实 legacy 副本自动
  迁移（缺集合按空导入 + 5 孤儿 turn 跳过计数 + JSON 原样 + 备份）、env=json 显式
  回退零 sqlite 产物、重启幂等（AlreadyCutover）。

## 存储边界（Gate 7 后）

默认行为：**SQLite 权威**。决议 marker-first：

- 有效 sqlite marker → SQLite（env=json 显式 fail-closed，绝不静默回退）。
- JsonAuthoritative marker（reverse-cutover 产物）→ JSON 权威；env=sqlite 是合法全新 opt-in 可重跑 cutover。
- 无 marker → 默认 SQLite：完整 legacy JSON 树自动迁移；目录无任何 legacy 布局文件 = 全新用户，初始化空 SQLite 权威。
- stale marker → 无论 env 都拒绝。

Gate 7 兼容语义（RESULT §36）：

- 缺失 legacy 文件按空集合导入（与 JSON store `load_or_default` 同口径）；存在但损坏仍 fail-closed。
- 孤儿行（父 campaign/conversation 已删除的残留，JSON 应用里不可达）跳过并计数（`skipped_orphan_rows`）。
- 不双写、不删除旧 JSON、不遇错静默建空库；每次 cutover 生成 sqlite-backups/ 备份检查点。
- JSON 保留为限期兼容导入、反向导出和紧急回退能力（保留至少一个发布周期）。

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

## 历史验证记录（保留）

- 2026-07-14：fmt/clippy/workspace test/frontend 全绿；真实模型与 OS credential store 用例按设计 ignored。
- 2026-07-22：前端三层解耦（design/adapter/stores），AppFrame 生产切换完成，`npm test` 317 / `test:ui` 35 / build 通过；未改 plugin-bridge / tauri-api 协议。
- 2026-08-05（Gate 7）：`cargo test --workspace`、§11.1 目标套件、前端 IPC 合同 8/8、`verify-release.ps1` Release gate passed。

## 下一优先级

1. Full100 真实模型续跑（Gate 6 唯一剩余阻塞；relay 恢复后 `run-stage.sh full native 100 3500`，预计 ~10 小时）。
2. ~~Gate 8 文档封存收尾~~ 已完成（2026-08-05，RESULT §37；本清单即封存产物之一）。
3. 发布后：Gate 7 完整候选周期统计；稳定期后另立计划删除 JSON 生产写路径（§12.1.5）。
4. Windows runner、Android 真机、签名包（需证书）与真实第三方插件验收。
5. CoT 三臂 × 80 轮（与 SQLite 收口分账，PLAN §16 排期）。

## 交接约束

- 不把真实 API key 写入仓库、文档、日志、截图或证据 JSONL（本线程曾明文暴露的 key 建议轮换）。
- 不把模型最大能力窗口等同于每次请求应设置的输出长度；`max_tokens` 是 ceiling，不是目标长度。
- 不把 deterministic fixture、可执行入口或 synthetic Chronicle 写成完整生产验收。
- 默认后端已是 SQLite；必须保留 fail-closed cutover、备份、reverse export 与显式 JSON 回退（至少一个发布周期）。
- 不 amend 既有历史提交；不 push；Gate 6 未 seal（Full100 BLOCKED）前不得把 Gate 6 写成 PASS。
- `docs/archive/**` 与旧 workstream PLAN/RESULT 是历史证据，除非修正事实错误，否则不回写成当前状态。

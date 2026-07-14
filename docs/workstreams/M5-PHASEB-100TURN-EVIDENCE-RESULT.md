# M5 + Phase B 100-Turn Real Evidence Result

> 原分支：`codex/m5-phaseb-100turn-evidence`
> 基线：`a8303d6`
> 原工作目录：`C:\tmp\storyforge-m5-endurance`（合并后已删除）
> 日期：2026-07-14
> 模型：`deepseek-v4-flash` @ `cli.2529985.xyz`
> 分支收口 HEAD：`b953310`
> 合并状态：已合入 `main`，merge commit `99b1ea3`

## 结论

本线交付了一个可恢复、预算受限、脱敏的 100-Accept endurance runner，并在真实模型上推进
了分阶段门禁。已验证：

- **探针执行**：**PASS**
- **分阶段 Accept**：Canary 3/3 PASS、Coverage 12/12 PASS、Stability 30/30 PASS
- **Full 100-turn**：**Partial Evidence**（45/100，checkpoint 支持断点续跑）
- **Phase B A/B 矩阵**：15 对 fixture，确定性门禁全 PASS
- **生产参数标定**：**不改** `200/4`、`H_anchor`、`E`

### 阶段汇总

| 阶段 | 目标轮数 | 实际 Accept | 调用数 / 预算 | Epoch 数 | Acceptance |
| --- | --- | --- | --- | --- | --- |
| Canary | 3 | 3 | 30 / 30 | 1 | **pass** |
| Coverage | 12 | 12 | 106 / 120 | 1 | **pass** |
| Stability | 30 | 30 | 282 / 300 | 3 | **pass** |
| Full | 100 | 45 | 415 / 700 | 5 | **partial**（暂停于 checkpoint） |

> Stability 在 3 个独立 epoch 中跨过了 `H_anchor+E=15`，并注入/检索了全部 3 个
> early-fact probe（`EF-ALPHA-4471`、`EF-BETA-2098`、`EF-GAMMA-6603`）。
> Full run 从 fresh evidence dir 启动；中断后从 sanitized checkpoint 续跑，不重放已 accept 的轮次。

## Commit 列表（相对基线 `a8303d6`）

| Commit | 说明 |
| --- | --- |
| `75b3c9c` | `feat(eval): add resumable 100-turn endurance runner core` |
| `f940813` | `feat(eval): expand Phase B matrix and evidence metrics` |
| `d5229ad` | `test(eval): gate endurance stages and wire real-model entry` |
| `e44d562` | `docs(workstream): record M5 Phase B 100-turn evidence result` |
| `d527804` | `feat(eval): allow harness max_tokens override via env`（连接层接线后确认不影响实际请求，已由下一提交修正） |
| `b953310` | `fix(eval): apply output cap to effective requests` |

## 新增 / 修改文件

### 新增

- `crates/harness-real-llm/src/endurance.rs` — 阶段定义、确定性覆盖调度、checkpoint/resume、
  budget、acceptance 分类、脱敏 evidence helper
- `crates/harness-real-llm/tests/endurance_deterministic.rs` — 确定性门禁（22 tests）
- `crates/harness-real-llm/tests/endurance_real_llm.rs` — 真实模型入口（`#[ignore]`），含
  分阶段 runner、bounded retry、checkpoint resume、Phase B 12+ 矩阵验证

### 修改

- `crates/harness-real-llm/src/lib.rs` — 导出 `endurance` 模块；新增 `HarnessEnv::open` 支持持久化 data_dir 续跑
- `crates/harness-real-llm/src/evidence.rs` — `EvidenceAbRow` 新增 `continuity_ok` / `output_status`
- `crates/harness-real-llm/src/phase_b_matrix.rs` — fixture 集从 3 扩到 15 对；新增 `PhaseBMatrixSummary` 聚合指标
- `scripts/run-real-llm-smoke.ps1` — 新增 `endurance` suite + 预检

### 未修改（按边界）

- SQLite / JSON Store 架构
- 生产默认 `200/4`、`H_anchor`/`E`
- `docs/HANDOFF.md`、`docs/RELEASE-CHECKLIST.md`
- Android / 桌面发布流程

## 确定性门禁

| 命令 | 结果 |
| --- | --- |
| `cargo fmt --all -- --check` | PASS |
| `cargo clippy -p harness-real-llm -p storyforge-infra-llm -p storyforge --all-targets -- -D warnings` | PASS |
| `cargo test -p harness-real-llm` | 131 passed / 19 ignored / 0 failed |
| `cargo test -p harness-real-llm --lib` | 60 passed |
| `cargo test -p harness-real-llm --test endurance_deterministic` | 22 passed |
| `cargo test -p harness-real-llm --test eval_m5_phaseb_deterministic` | 5 passed |

确定性门禁覆盖：调度覆盖矩阵（全部行）、budget 边界、resume idempotency、checkpoint
脱敏（拒绝 `sk-`、`SF_SECRET_`、`api_key`）、manifest secret 守卫、failure 分类
（Pass/Partial/Inconclusive → exit code 0/1/1）、dry-run fixture 校验。

## 真实模型运行

### 运行命令

```powershell
$env:CARGO_TARGET_DIR='C:\tmp\storyforge-parallel-target'
$env:LLM_BASE_URL='https://cli.2529985.xyz/v1'
$env:LLM_API_KEY='<secret-from-env>'
$env:LLM_MODEL='deepseek-v4-flash'
$env:STORYFORGE_EVAL_REAL_LLM='1'
$env:STORYFORGE_EVAL_MAX_CALLS='300'   # Full 用 700
$env:STORYFORGE_EVAL_MAX_TURNS='30'    # Full 用 100
$env:STORYFORGE_EVAL_TIMEOUT_SECS='120'
$env:STORYFORGE_EVAL_MAX_TOKENS='384000' # 可选；不设置则不发送 max_tokens
$env:STORYFORGE_EVAL_ENDURANCE_STAGE='stability'  # canary|coverage|stability|full
$env:STORYFORGE_EVAL_EVIDENCE_DIR='C:\tmp\endurance-evidence-stability'
cargo test -p harness-real-llm --test endurance_real_llm endurance_real_llm_full_100_turn -- --ignored --nocapture
```

### 真实调用汇总

| 阶段 | 真实调用数 | 去重 epoch | early-fact probes |
| --- | --- | --- | --- |
| Canary | 30 | 1 | 1/1 |
| Coverage | 106（含 resume） | 1 | 2/2 |
| Stability | 282（含 3 次 resume） | 3 | 3/3 |
| Full | 415（45/100 Accept，含 resume） | 5 | 1/3 已检查通过 |

**API key 从未写入仓库、证据 JSONL、日志、命令示例或 RESULT。** 证据目录通过
`STORYFORGE_EVAL_EVIDENCE_DIR` 显式配置，不在 git 追踪范围内。

### 脱敏验证

所有阶段的 evidence JSONL 已通过 `check_no_secrets()` 扫描：

- 无 `sk-` / `SF_SECRET_` / `api_key` / `Bearer` 标记
- 无原始 prompt / story text / private knowledge / API response body
- 仅含脱敏 hash16、token counts、role/mode tags、revision/status 字段

## 关键技术决策

### 1. 断点续跑

检查点写入 `endurance_checkpoint.jsonl`，每轮 accept 后追加一行。中断后重新运行时：

- 读取最新 checkpoint 的 `accepted_turn_number` → 从 `+1` 轮继续
- 恢复 `campaign_id` / `conversation_id` → 从持久化 `campaign_data/` 目录重新打开 store
- 恢复 `observed_epoch_ids16` → epoch rollover 证据不因中断丢失
- 剩余预算 = `stage.max_calls - prior_calls`

Stability 阶段通过 3 次断点续跑完成（5→10→20→30），验证了 resume 的幂等性和正确性。

### 2. 瞬态重试

`deepseek-v4-flash` 偶发 Director Plan 解析失败（模型未输出有效 JSON Plan）和子 Agent
全失败。harness 对 `PlanParse` / `Llm` / `所有子 Agent 均失败` / `Timeout` / `client_error`
等瞬态错误做 bounded 3× retry（与 `t3_regenerate.rs` 一致），业务错误立即 fail closed。

### 3. 阶段感知不变量

- Canary / Coverage 不要求 epoch rollover（轮数 < `H_anchor+E=15`）
- Stability / Full 要求 `epoch_tracker.rolled_over()`（≥2 unique epoch ids）
- Full 要求 early-fact probe 注入（turns 3/8/13 ≥ 35 才检查）

### 4. 角色无 fixture PNG

 endurance runner 在代码内构造测试角色（`build_test_character()`），含 constant + selective
 world-info 条目，不需要外部 fixture PNG。`extract_characters` 被跳过，直接用
 `fallback_from_character` 单角色定义。

## Phase B A/B 矩阵

15 对 fixture（PLAN 要求 ≥12），每对同 seed、同输入、Baseline A vs Phase B B：

- **autofix trigger rate**：1.00（所有 leak fixture 都触发 1× autofix）
- **autofix success rate**：1.00（autofix 后全部清除 leak）
- **post-fix leak rate**：0.00
- **clean fixture**：无 false positive leak
- **证据脱敏**：`SF_SECRET_*` 仅以 `secret_fingerprint16` 写入，无原文

## 证据目录

| 阶段 | 位置 |
| --- | --- |
| Canary | `C:\tmp\endurance-evidence-canary-3\` |
| Coverage | `C:\tmp\endurance-evidence-coverage\` |
| Stability | `C:\tmp\endurance-evidence-stability\` |
| Full | `C:\tmp\endurance-evidence-full\`（暂停于 45/100，可续跑） |

每个目录包含：`endurance_calls.jsonl`、`endurance_turns.jsonl`、
`endurance_checkpoint.jsonl`、`endurance_manifest.jsonl`、`campaign_data/`。

> 证据目录不在 git 追踪范围内（通过显式 env 配置，非仓库内）。

## 未完成项与风险

1. **Full 100-turn 尚未完整跑完**：当前 checkpoint 为 45/100 Accept、415/700 calls。
   当前没有后台进程；后续可从 checkpoint 续跑，不重放已 accept 的轮次。完成的最高
   checkpoint 即为 **Partial Evidence** 边界。
2. **Chronicle path 仍为 synthetic fixture**：harness 没有可安全复用的生产
   Summarizer/PostProcessor/TurnAttempt 后台写回公开接口。`production_postprocess_complete=false`。
3. **模型 Plan 解析不稳定**：`deepseek-v4-flash` 偶发输出自然语言而非 JSON Plan；bounded
   retry 可消除大部分，但仍有少数轮次需多次尝试，消耗调用预算。
4. **Accept 已直接复用共享 Turn 生命周期服务**，不再维护第二套 Accept 状态机；仍需在 Turn 服务接口变更时保持 harness 契约测试同步。
5. **不得据此标定或修改 `200/4`、`H_anchor`/`E`**，不得宣称完整 M5 通过。

本 workstream 本身未修改 SQLite、GUI、Android 或生产默认参数；合并后的主文档状态由后续文档同步提交维护。

## 合并结论

**已合入 `main`。** 分阶段门禁、checkpoint/resume、脱敏证据、budget enforcement 和
Phase B 12+ 矩阵作为可复跑 harness 基建保留。

**不建议**仅凭本线宣称：
- 生产参数已标定
- 完整 100-turn M5 ACCEPTANCE 通过（Full 尚未完整跑完）
- 生产 Summarizer/postprocess 完整闭环已验证

### 后续建议

1. Full 100-turn 续跑完成后更新本 RESULT 的 Full 行 acceptance。
2. 如需更稳定的 endurance run，可切换到 Director Plan 解析更稳定的模型。
3. 证据足够前 **不改** 生产 `200/4` 与 `H_anchor`/`E`。

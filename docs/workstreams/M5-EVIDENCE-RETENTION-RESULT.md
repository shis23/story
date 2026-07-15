# M5 Evidence Retention Result

> 分支：`codex/m5-evidence-retention`
> 基线：`945ba68`（plan commit）/ 上游 `8732e21`
> 工作目录：`C:\tmp\storyforge-m5-evidence-retention`
> 日期：2026-07-15
> 类型：**确定性证据基建**；本线 **0 次真实/付费模型调用**

## 结论

本线交付了下一次 M5 真实运行所需的**脱敏、持久、可离线校验**证据保存机制，只验证确定性基建，不产生也不伪造真实模型证据。

| 项 | 状态 |
| --- | --- |
| 唯一 controlled run ID（`run-<stage>-<uuid>`） | **PASS**（确定性） |
| 原子 `run_manifest.json` + 文件 SHA-256 | **PASS**（确定性） |
| archive / restore staging → verify → atomic rename | **PASS**（确定性） |
| retention plan/apply 完整 verify manifest/status/run_id/hash | **PASS**（确定性） |
| fail-closed resume：reparse 递归拒绝先于任何 checkpoint/body 打开 | **PASS**（确定性） |
| adversarial resume：外部 `campaign_data`/checkpoint link 零读写 | **PASS**（可用平台） |
| unsealed Interrupted resume：checkpoint integrity baseline 强制 | **PASS**（确定性） |
| exact-set verify（遗漏 / 重复 / 额外 JSONL） | **PASS**（确定性） |
| secret scan（含 hidden / `.tmp` / nested snapshot） | **PASS**（确定性） |
| seal/verify 失败使真实 Full run 硬失败（非 warning） | **PASS**（代码路径；真实 run 未执行） |
| 路径与错误消息不回显凭证 | **PASS**（确定性） |
| 真实 Full 100-Accept 运行 | **未执行** |
| 旧 45/100 checkpoint 恢复 | **不可恢复；本线不伪造** |

**本线只验证确定性证据基建。** 它不能替代下一次真实 M5 Full run，也不能把已清理的 45/100 Partial Evidence 复活。

## 交付内容

### 新增 / 强化

- `crates/harness-real-llm/src/evidence_retention.rs`
  - durable evidence root 策略（repo / live data / temp 默认拒绝）
  - `resolve_resume_run_dir` / `assert_tree_safe_for_resume`：resume 前递归拒绝 reparse/junction/symlink，并校验将打开路径的 canonical containment；**不读文件正文**
  - `checkpoint_integrity.json` baseline：未 seal 的 Interrupted resume 必须有完整性基线，否则不得称为可审计 fail-closed resume
  - exact-set digest verify、全树 secret scan
  - archive/restore：sibling `.staging-*` → verify → rename 发布；失败清理 stage，可重试
  - retention plan/apply：仅对完整 `verify_run` 通过且 status∈{completed,archived} 的 controlled run 操作；apply 再 verify，拒绝状态漂移/无 manifest/hash 篡改
- `crates/harness-real-llm/tests/evidence_retention_deterministic.rs`（36 tests）

### 修改

- `crates/harness-real-llm/src/endurance.rs`
  - `resume_from_evidence_dir`：**先** tree safety，**后** checkpoint body
  - durable root `from_env` / dry-run root 策略
- `crates/harness-real-llm/tests/endurance_deterministic.rs` — resume + baseline
- `crates/harness-real-llm/tests/endurance_real_llm.rs`
  - resume dir 全量策略校验
  - 每 accept 刷新 checkpoint integrity baseline
  - seal/verify 硬失败
- `scripts/run-real-llm-smoke.ps1` — evidence root 预检

### 明确未改

- `crates/tauri-app/**` 业务、`crates/infra-sqlite/**`
- `docs/HANDOFF.md`、`docs/RELEASE-CHECKLIST.md`、历史 RESULT 数字
- M5 模型/预算/acceptance/Context 参数
- 未读取/写入真实凭证；未调用真实模型

## Resume / archive 语义（收口）

1. **Resume 顺序（P0）**
   递归 reparse/containment（metadata only）→（若 unsealed）checkpoint integrity baseline → 再打开 checkpoint / 可选 `campaign_data`。
   禁止在安全验证前 `read_latest_checkpoint`。

2. **Unsealed Interrupted**
   仅在存在 `checkpoint_integrity.json` 且 hash 匹配时，才作为**可审计 fail-closed resume**。
   无 baseline = `CheckpointIntegrity` / resume unavailable。

3. **Sealed Interrupted**
   允许通过 `verify_run` + status 读取 resume 上下文；**不可 archive**。

4. **Archive**
   仅 completed/archived 审计包；staging 发布；永不包含 live `campaign_data`；不是续跑载体。

5. **Retention**
   plan 与 apply 都完整 verify；无 manifest / hash 漂移 / status 漂移一律不可删。

## 环境变量

| 变量 | 含义 |
| --- | --- |
| `STORYFORGE_EVAL_EVIDENCE_ROOT` | 首选持久 multi-run root（repo 外；默认拒 temp） |
| `STORYFORGE_EVAL_EVIDENCE_DIR` | 兼容 resume run dir 或 root；resume 时 parent 必须通过 durable 策略 |
| `STORYFORGE_EVAL_ALLOW_EPHEMERAL_EVIDENCE=1` | 显式允许 temp（测试） |
| `STORYFORGE_REPO_ROOT` | 可选 repo root 覆盖 |

## 确定性门禁

| 命令 | 结果 |
| --- | --- |
| `cargo fmt --all -- --check` | PASS |
| `cargo test -p harness-real-llm --lib` | PASS |
| `cargo test -p harness-real-llm --test evidence_retention_deterministic` | PASS（36） |
| `cargo test -p harness-real-llm --test endurance_deterministic` | PASS（25） |
| `cargo test -p harness-real-llm --test eval_m5_phaseb_deterministic` | PASS（5） |
| `cargo clippy -p harness-real-llm --all-targets -- -D warnings` | PASS |
| `git diff --check` | PASS |
| 真实 LLM / ignored tests | **未执行** |

## 旧 45/100 边界

2026-07-14 Full 的 45/100 是历史 Partial Evidence。原始 temp evidence/`campaign_data` 已清理，**不能续跑或独立重放**。本线不伪造 JSONL/checkpoint/manifest，也不更新该次 acceptance。

## 剩余风险 / 下一步

1. 新的真实 Full 100-Accept run 必须使用 durable root，并依赖本线 seal/verify/archive。
2. 无特权 Windows 环境可能无法创建 symlink 测试夹具；实现路径仍强制 reparse 拒绝。创建成功时 adversarial 测试证明外部 payload 零读写。
3. Retention CLI 包装可后续单独添加。

## 合并建议

**可以合并到评估栈作为证据基建。**
合并后仍不得把 M5 Full 标为完成，直到新的真实 100-Accept 证据在 durable root 上通过 offline verify。

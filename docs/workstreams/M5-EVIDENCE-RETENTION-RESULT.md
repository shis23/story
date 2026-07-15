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
| archive / restore / re-hash | **PASS**（确定性） |
| retention 命名空间 + active 保护 + 路径逃逸拒绝 | **PASS**（确定性） |
| fail-closed resume（缺 checkpoint / 混 run / hash / schema） | **PASS**（确定性） |
| 路径与错误消息不回显凭证 | **PASS**（确定性） |
| 真实 Full 100-Accept 运行 | **未执行** |
| 旧 45/100 checkpoint 恢复 | **不可恢复；本线不伪造** |

**本线只验证确定性证据基建。** 它不能替代下一次真实 M5 Full run，也不能把已清理的 45/100 Partial Evidence 复活。

## 交付内容

### 新增

- `crates/harness-real-llm/src/evidence_retention.rs`  
  - evidence root 策略（拒绝 repo 内、live `data/`、默认拒绝 temp/ephemeral）  
  - `STORYFORGE_EVAL_EVIDENCE_ROOT`（首选）/ 兼容 `STORYFORGE_EVAL_EVIDENCE_DIR`  
  - 唯一 run ID、exclusive run dir、原子 manifest、hash、seal/verify  
  - archive/restore（仅复制 digest 列表内相对路径；不封 `campaign_data`）  
  - retention plan/apply（只删 controlled 命名空间内 completed/archived，保护 active）  
  - path traversal / outside-root / mixed-run fail-closed
- `crates/harness-real-llm/tests/evidence_retention_deterministic.rs`（20 tests）

### 修改

- `crates/harness-real-llm/src/lib.rs` — 导出 `evidence_retention`
- `crates/harness-real-llm/src/endurance.rs`  
  - `EnduranceRunConfig::from_env` 走校验后的 durable root + controlled run dir  
  - `resume_from_evidence_dir` fail-closed  
  - dry-run 增加 `evidence_root_ok` / retention schema 检查，路径显示脱敏
- `crates/harness-real-llm/tests/endurance_deterministic.rs` — dry-run / resume 门禁扩展
- `crates/harness-real-llm/tests/endurance_real_llm.rs`  
  - 文档与入口改用 durable root / controlled run dir  
  - resume 预检 fail-closed；成功后 seal+verify（仍 `#[ignore]`，本线未跑）
- `scripts/run-real-llm-smoke.ps1` — endurance suite 增加 evidence root 预检

### 明确未改

- `crates/tauri-app/**`（业务逻辑）、`crates/infra-sqlite/**`
- `docs/HANDOFF.md`、`docs/RELEASE-CHECKLIST.md`、历史 RESULT 数字
- M5 `max_tokens` / 模型 / 预算 / acceptance 门槛 / 生产 Context 参数
- 未读取、未写入真实凭证；未调用真实模型

## 环境变量约定（文档化默认保守）

| 变量 | 含义 |
| --- | --- |
| `STORYFORGE_EVAL_EVIDENCE_ROOT` | **首选** 持久 multi-run root（须在 repo 外；默认拒绝 temp） |
| `STORYFORGE_EVAL_EVIDENCE_DIR` | 兼容：已有 controlled run dir（含 checkpoint）用于 resume，或作为 root |
| `STORYFORGE_EVAL_ALLOW_EPHEMERAL_EVIDENCE=1` | 显式允许 temp/ephemeral（仅测试/临时）；生产默认关闭 |
| `STORYFORGE_REPO_ROOT` | 可选，覆盖 repo root 探测 |

未设置 durable root 且未允许 ephemeral 时，`from_env` / smoke preflight **fail closed**。

## 确定性门禁

| 命令 | 结果 |
| --- | --- |
| `cargo fmt --all -- --check` | PASS |
| `cargo test -p harness-real-llm --lib` | PASS（63） |
| `cargo test -p harness-real-llm --test evidence_retention_deterministic` | PASS（20） |
| `cargo test -p harness-real-llm --test endurance_deterministic` | PASS（25） |
| `cargo test -p harness-real-llm --test eval_m5_phaseb_deterministic` | PASS（5） |
| `cargo clippy -p harness-real-llm --all-targets -- -D warnings` | PASS |
| `git diff --check` | PASS |
| 真实 LLM / ignored tests | **未执行** |

## 旧 45/100 边界（再次确认）

2026-07-14 Full 的 45/100 是历史 Partial Evidence。四个 `C:\tmp\endurance-evidence-*` 与原始 `campaign_data` 已清理，**不能续跑或独立重放**。本线：

- 不伪造 JSONL / checkpoint / manifest 来“补” 45/100；
- 不为该次运行声称新的 acceptance；
- 只为**下一次**新的 Full 100-Accept run 提供可保留、可校验、可归档的证据链。

## 剩余风险 / 下一步

1. 需要一次**新的**真实 Full run，使用 durable `STORYFORGE_EVAL_EVIDENCE_ROOT`，完成后依赖本线 seal/verify/archive。  
2. 若要把 retention cleanup 做成独立 CLI/二进制入口，可在本模块 API 上包一层，无需再改 schema。  
3. Windows reparse/junction 删除 hardening 目前依赖 controlled 命名空间 + outside-root 检查；若 evidence root 本身被换成 junction，运维侧仍应避免。

## 合并建议

**可以合并到评估栈作为证据基建。**  
合并后仍不得把 M5 Full 标为完成，直到新的真实 100-Accept 证据在 durable root 上通过 offline verify。

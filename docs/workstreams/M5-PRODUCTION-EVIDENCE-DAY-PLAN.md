# M5 生产证据线 Day Plan

> 分支：`codex/m5-production-evidence`
> 基线：`main@053847f`
> 工作目录：`C:\tmp\storyforge-m5`
> 日期：2026-07-13

## 范围

本线只补当前 M5 可在本地完成的最大缺口：把已经存在的真实模型单轮 smoke、
`BudgetedLlmClient`、生产忠实 `CommitTurn`/Accept 探针、ContextEpoch 计算与脱敏
JSONL writer 串成一个可复跑的多轮生产证据入口。

交付范围：

1. 真实模型入口执行连续多轮 `write → CommitTurn/Accept`，每轮使用同一 Campaign、
   Conversation 与存储，下一轮重新走生产 Context 装配。
2. 接受轮数必须严格跨越 `H_anchor + E`，并证明 ContextEpoch 已发生 rollover/成员变化，
   不能仅用“轮数大于阈值”替代真实 epoch 证据。
3. 全 suite 共用同一个 `BudgetedLlmClient`，`max_calls` 是硬上限，`timeout_secs` 对每次
   调用生效；预算耗尽或调用超时必须失败。
4. 每次模型调用和每个 Accept 轮分别写脱敏 JSONL；只记录 hash、usage、耗时、状态、
   revision、epoch 与断言，不记录完整 prompt、完整响应、API key 或私密探针原文。
5. fixture 缺失、真实调用数为零、未完成目标 Accept 数、未跨 epoch、证据写入失败均
   fail closed，不得输出成功结论。
6. 真实用例保持 `#[ignore]`；本线不调用任何付费模型。使用确定性 writer / 假 LLM
   验证 orchestration、suite-wide 预算、redaction、epoch rollover 与 JSONL 落盘。

## 禁止项

- 不修改 SQLite、JSON Store 架构或生产存储接线。
- 不修改 Tauri GUI、桌面/Android 发布流程或 `docs/HANDOFF.md`。
- 不修改生产默认 `overview_max_entries=200`、压缩阈值/分组 `200/4`、`H_anchor=5`、
  `E=10`。
- 不复制第二套长期存在的 evidence schema；沿用 `eval-m5-phaseb-v1` call/turn 记录。
- 不把确定性测试或真实入口可执行写成完整 M5 参数标定通过。
- 不运行付费模型；不提交凭证、完整 prompt、完整供应商响应或未脱敏正文。
- 不跑全 workspace 门禁；只运行本线约定的 harness 专项验证。

## 实施顺序

1. 先写确定性失败测试：预算不足、零调用、未跨 epoch、证据泄漏/落盘失败。
2. 新增最小多轮 runner，复用 `HarnessEnv`、`CommitProbe`、`BudgetedLlmClient`、
   `EvidenceWriter` 与生产 Context 装配。
3. 将真实 `#[ignore]` 测试从单轮入口升级为多轮生产证据入口。
4. 只调整 `scripts/run-real-llm-smoke.ps1` 的 eval 区段，使默认预算与提示符合跨 H+E
   的真实运行前置条件。
5. 运行专项格式、测试、clippy 与 diff 检查，新增独立 RESULT 文档并提交。

## 验收

- `cargo fmt --all -- --check`
- `cargo test -p harness-real-llm --lib`
- `cargo test -p harness-real-llm --test eval_m5_phaseb_deterministic`
- 新增多轮 orchestration 专项测试全部通过；真实模型测试仍为 ignored。
- `cargo clippy -p harness-real-llm --all-targets -- -D warnings`
- `git diff --check 053847f..HEAD`
- 自动测试证明：
  - suite-wide `max_calls` 不可超支，timeout 返回失败；
  - 每次调用与每轮 Accept 都有 JSONL；
  - JSONL 不含 API key、完整 messages/prompt、`SF_SECRET_*` 原文；
  - 接受轮数 `> H_anchor + E`，且 epoch id/成员确实 rollover；
  - fixture 缺失、零调用、未跨 epoch 均返回失败。
- 真实模型调用次数为 `0`，RESULT 明确列出真实运行命令与仍未完成的真实证据。

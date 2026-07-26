# M5 与 Phase B 评估线计划

## 基线与边界

- 分支：`codex/eval-m5-phaseb`
- 基线：`7fb1899`
- 工作目录：`C:\tmp\storyforge-eval`
- 本线只负责真实模型评估 harness、脱敏证据和评估报告。
- 不修改 SQLite、Campaign/Turn 存储实现、桌面发布流程或 Android。
- 不直接更新 `docs/HANDOFF.md`；最终结果写在本文件或同目录的新报告中。

## 目标

1. 增加生产 `CommitTurn`/Accept 闭环探针，不再手工写 Chronicle A。
2. 增加至少 20 个 Accept 轮的长会话测试，跨越 `H_anchor + E`。
3. 将每次调用的脱敏证据写入 JSONL，包括角色、streaming、segment hash、usage、耗时和断言结果。
4. 建立 Phase B 前后对照矩阵：泄漏率、质量警告、自动修复次数、延迟和 token 成本。
5. 保持 `200/4`、`H_anchor/E` 为未标定参数，除非真实证据足够。

## 实施顺序

1. 审计现有 `harness-real-llm`、M5 S1-S6 与 production CommitTurn 入口。
2. 先实现无需付费模型即可编译和执行的 deterministic/mock 闭环测试。
3. 为真实模型运行增加显式开关、预算上限、超时和脱敏 JSONL writer。
4. 新增生产 Accept 探针；断言正文、active variant、Attempt hash、Campaign revision、Chronicle A 和 epoch 成员。
5. 新增长会话 runner；每轮保存 token、cache、延迟、错误和早期事实可达性。
6. 新增 Phase B A/B fixture，A/B 必须使用同一输入、模型参数与随机种子。
7. 未经用户明确授权，不调用付费模型。

## 可修改区域

- `crates/harness-real-llm/**`
- `scripts/run-real-llm-smoke.ps1`
- M5/Phase B 专用测试 fixture
- `docs/workstreams/**`
- 必要的脱敏观测接口；修改生产代码前必须说明原因并添加回归测试

## 禁止事项

- 不修改 SQLite 或 JSON Store 架构。
- 不把探针执行成功写成完整 M5 验收通过。
- 不提交 API key、完整 prompt、完整 secret 或原始供应商响应。
- 不更改生产默认压缩参数。
- 不长期双写新的证据格式。

## 验收门槛

- `cargo fmt --all -- --check`
- `cargo test --workspace --quiet`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `git diff --check 7fb1899..HEAD`
- 无凭证时真实 LLM 测试应明确 ignored，而不是假通过。
- JSONL 必须经过自动脱敏测试。

## 最终交付

结束时在本目录新增 `EVAL-M5-PHASEB-RESULT.md`，记录：

- commit 列表与修改文件
- 实际执行的测试和真实模型调用次数
- 费用/耗时（若实跑）
- 可独立审计的证据位置
- Pass / Partial Evidence / Inconclusive 分项结论
- 是否建议合并及剩余风险

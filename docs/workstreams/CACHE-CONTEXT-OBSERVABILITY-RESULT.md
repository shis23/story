# Cache and Context Observability Result

> 分支：`codex/cache-context-observability`
> 基线：`main@c3a972d`
> 日期：2026-07-13
> 工作目录：`C:\tmp\storyforge-cache-context`
> HEAD：见本文件提交后 `git rev-parse HEAD`

## 1. Commit 列表（相对基线）

| Commit | 说明 |
|--------|------|
| `543f0a1` | docs(workstream): plan cache context observability |
| `db83e1f` | docs(workstream): authorize capped real cache validation |
| `f766be3` | feat(llm): unify provider usage parser matrix |
| `9578310` | feat(layout): add LCP and reusable prefix token estimates |
| `25e2fde` | feat(harness): record segment hashes with boot/turn sample split |
| `1151030` | test(harness): add 100+ context compile benchmark and observability gates |
| `f52b533` | fix(llm): tighten zero-cached usage assertion for clippy |
| *(本 RESULT 提交)* | docs(workstream): CACHE-CONTEXT-OBSERVABILITY-RESULT |

## 2. 修改文件

### 新增

- `crates/infra-llm/src/usage_parse.rs` — OpenAI/DeepSeek/Anthropic usage 统一解析 + 表驱动矩阵
- `crates/harness-real-llm/src/observability.rs` — boot/turn 分类、LCP/可复用 token、模拟前缀 cache、报告写入
- `crates/harness-real-llm/src/context_compile_bench.rs` — 120 轮确定性 Context 编译基准
- `crates/harness-real-llm/tests/cache_context_observability.rs` — 专项门禁
- `docs/workstreams/CACHE-CONTEXT-OBSERVABILITY-PLAN.md`（计划，既有）
- `docs/workstreams/CACHE-CONTEXT-OBSERVABILITY-RESULT.md`（本文件）

### 修改

- `crates/infra-llm/src/lib.rs` — 导出 `usage_parse`
- `crates/infra-llm/src/openai.rs` / `sse.rs` — 流式/非流式共用解析路径
- `crates/domain/src/message_layout.rs` — `estimate_tokens_approx` / LCP / reusable prefix tokens
- `crates/harness-real-llm/src/lib.rs` — 导出 observability / context_compile_bench

### 未修改（按边界）

- 生产默认 `overview_max_entries=200`、`compress 200/4`、`H_anchor=5`、`E=10`
- Turn lifecycle / SQLite / plugin / release 行为
- `docs/HANDOFF.md`
- 未 push

## 3. 红测 → 绿测证据

| 步骤 | 证据 |
|------|------|
| RED 意图 | 需要表驱动 usage 矩阵、LCP/可复用 token、boot 分离、模拟 cache、100+ 编译预算与脱敏报告 |
| 实现前缺口 | 流式/非流式 cache 字段各自解析；无统一 matrix；无 LCP/reusable estimate API；无 boot/turn 归因隔离模块；无 100+ 编译基准报告 |
| GREEN | `usage_parse` 矩阵 13 case + vacuous-zero 防护；message_layout LCP 测试；observability 模拟 cache / 样本分区；120 轮 bench 全断言通过 |
| Clippy 红→绿 | `nonminimal_bool` 于 zero-cached 断言 → 改为 `assert_eq!(cached, 0)` |

## 4. Schema coverage（usage parser）

覆盖：

- OpenAI nested `prompt_tokens_details.cached_tokens`（非流式 + 流式末 chunk）
- DeepSeek 顶层 `prompt_cache_hit_tokens` / `prompt_cache_miss_tokens`（优先于 nested）
- Anthropic `cache_read_input_tokens` / `cache_creation_input_tokens`
- partial stream（仅 prompt + nested cache）
- final full usage
- missing usage / missing prompt_tokens → `None`
- 畸形：字符串数字、null cache、负 cache → 降级 0
- non-object usage → `None`
- **显式禁止** `0 >= 0` 式 cache 成功条件：`cached_tokens==0` 仅表示无命中，不构成成功

## 5. Benchmark data（确定性，无网络）

命令：

```text
cargo test -p harness-real-llm --test cache_context_observability context_compile_benchmark_and_budget_assertions -- --nocapture
```

观测（本机一次运行）：

| 指标 | 值 |
|------|-----|
| turns | 120 |
| H_anchor / E / overview_max | 5 / 10 / 200 |
| max_near_raw cap | 15 |
| observed_max_near_raw | 14（≤15） |
| final near / band / overview | 14 / 10 / 96 |
| prompt_growth_slope | ≈ 6.65（预算 ≤ 120） |
| latency p50 / p95 | 0ms / 0ms（纯函数，亚毫秒） |
| evidence_bytes | ≈ 2427（预算 ≤ 256KiB） |
| 默认未变 | `200/4/H=5/E=10` 断言通过 |

说明：token 估计为 UTF-8 字节/4 粗粒度，**不是**供应商 billed token；报告**不**宣称 provider cache hits。

## 6. 脱敏证据

- `write_observability_report` / evidence writer 拒绝 `api_key` / `sk-` / `SF_SECRET_` / `"messages"` / 完整 prompt
- 基准报告正文不含合成 draft 全文与密钥
- 专项门禁断言：`!body.contains("api_key"|"SF_SECRET_"|"\"messages\"")`

## 7. 实际测试结果（PLAN 专项门禁）

| 门禁 | 结果 |
|------|------|
| `cargo test -p storyforge-infra-llm --lib` | PASS（50） |
| `cargo test -p storyforge-domain message_layout --lib` | PASS（14） |
| `cargo test -p harness-real-llm --lib --test cache_context_observability` | PASS（lib 22 + gate 5） |
| `cargo clippy -p storyforge-infra-llm -p storyforge-domain -p harness-real-llm --all-targets -- -D warnings` | PASS |
| `cargo fmt -p storyforge-infra-llm -p storyforge-domain -p harness-real-llm -- --check` | PASS |
| `git diff --check c3a972d..HEAD` | PASS |
| 全 workspace | **未跑**（按 PLAN 禁止） |

## 8. 真实 provider 调用

| 项 | 值 |
|----|----|
| 调用次数 | **0** |
| 原因 | 确定性门禁优先；本切片已完成本地可解释性证据。限量真实校验（`cli.2529985.xyz` / `grok-4.5` / `LLM_API_KEY` / ≤32 calls）**尚未执行** |
| 是否宣称 cache hit | **否** |

### 剩余真实 provider 问题（未 closure）

1. 该 endpoint 流式末 chunk 是否稳定带 `usage` / nested cache 字段。
2. 同 epoch 稳定 system 前缀在真实网关的 cache 信号是否与 segment hash 对齐。
3. boot/setup 调用是否会污染写作 turn 的 cache 归因（本地已隔离，真跑需核对 tag）。
4. 在预算与重试策略下，首次 schema/redaction 失败即停的可操作性。

## 9. 未完成项与风险

- 限量真实 LLM 校验未跑（需显式 env + 密钥，且不得把 key 写入证据）。
- 粗粒度 token 估计不能替代模型 tokenizer；斜率预算仅防失控增长。
- 模拟 cache 用 stable prefix fingerprint 事件机，证明的是**前缀稳定性语义**，不是供应商计费命中。
- `frontend/dist` 本地占位未入库（gitignore），仅用于本机通过 tauri `frontendDist` 编译 harness。

## 10. 是否建议合并

**建议合并（deterministic 切片）**。

理由：

1. PLAN 要求的本地可观测垂直切片已落地并通过专项门禁。
2. 生产 `200/4/H/E` 默认有硬断言，未改 Turn/SQLite/plugin。
3. 未宣称 provider cache hits；真实限量跑可作为 follow-up，不阻塞本 deterministic 证据合并。

不建议在本 PR 中夹带真实调用结果，除非另开授权运行并只记录 host/model/计数/hash。

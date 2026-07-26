# 2026-07-26 已完结 workstream 归档

> 归档执行依据：`docs/workstreams/ARCHITECTURE-REVIEW-2026-07-26.md` P2-11（文档减负）。
> 判定规则（保守）：工作流自身记录为已交付/已合并；不在 `docs/HANDOFF.md` 下一优先级中；无活跃文档引用其为现行权威。有任何疑问的一律保留在原处。
> 本目录文件为历史证据，除修正事实错误外不回写为当前状态（沿用 `docs/archive/**` 既有约束）。

## 归档清单（19 组 / 38 文件）

| 组 | 完结理由（一行） |
| --- | --- |
| EVAL-M5-PHASEB (PLAN/RESULT) | 2026-07-13 确定性探针 PASS，真实模型后续由 M5-PHASEB-100TURN-EVIDENCE（保留）接棒 |
| NIGHTLY-INTEGRATION (PLAN/RESULT) | 2026-07-13 集成事件完结，17 个 commit 全部合入 main |
| SQLITE-MIGRATION-FOUNDATION (PLAN/RESULT) | 基础 crate 交付合入；持久决策在 `docs/adr/0001`；现状归 SQLITE-CURRENT-STATUS-AUDIT |
| M5-PRODUCTION-EVIDENCE-DAY (PLAN/RESULT) | 编排 PASS/证据 Inconclusive 收口；权威结果由 100TURN 线（保留）承载 |
| SQLITE-PRODUCTION-UOW-DAY (PLAN/RESULT) | Turn-accept UoW 切片交付；后续项由 CHRONICLE-MIGRATION / OPTIN-BACKEND 线完成 |
| CACHE-CONTEXT-OBSERVABILITY (PLAN/RESULT) | 确定性观测切片交付合入；RESULT §8 的真实供应商问题无活跃载体（归档时知悉） |
| IMPORT-EXPORT-HARDENING (PLAN/RESULT) | 兼容矩阵/原子导入交付；fixture 后续由 IMPORT-FIXTURE-CORPUS 完成 |
| PLUGIN-COMPAT-MATRIX (PLAN/RESULT) | 可执行矩阵交付；机器可读权威在代码（pluginCompatMatrix.js / compat_matrix.rs） |
| RELEASE-BUILD-PIPELINE (PLAN/RESULT) | host 侧发布证据管线交付；runner 未了项归 RELEASE-CI-EVIDENCE / RUNNER-READINESS（保留） |
| SQLITE-CHRONICLE-MIGRATION (PLAN/RESULT) | B/C publication UoW + V3 迁移交付；cutover 由 opt-in 线完成 |
| TURN-LIFECYCLE-SERVICE (PLAN/RESULT) | 共享 Turn 生命周期服务交付合入 |
| IMPORT-FIXTURE-CORPUS (PLAN/PROMPT/RESULT) | P1 证据链修复全部落地，HANDOFF 已记为已落地主线 |
| PLUGIN-RUNTIME-FOLLOWUP (PLAN/PROMPT/RESULT) | prompt hook/权限撤销/预算/审计链落地；真实 iframe 人工验收归发布流程伞项 |
| SQLITE-OPTIN-BACKEND (PLAN/PROMPT/RESULT) | opt-in 后端完整实现（132 测试 + 故障注入）；现状归 SQLITE-CURRENT-STATUS-AUDIT |
| SQLITE-PREACCEPT-LIFECYCLE (PLAN/RESULT) | 自带 2026-07-21 supersession 说明，事实入口已重定向 |
| M5-EVIDENCE-RETENTION (PLAN/RESULT) | seal/verify/archive/retention 全 PASS；前向项归 SQLITE-M5-100-ENDURANCE（保留） |
| PRODUCTION-POSTPROCESS-SERVICE (PLAN/RESULT) | 共享服务落地为生产代码，HANDOFF 按符号引用而非按文档引用 |
| AGENT-PROMPTS | 2026-07-13 夜间并行跑的一次性提示词单（固定 worktree/过期基线），非活跃参考 |

## 保留在 `docs/workstreams/` 的活跃线

RELEASE-BRONZE（人工 GUI 验收未闭环，RELEASE-CHECKLIST 引用）· RELEASE-CI-EVIDENCE 与 RELEASE-RUNNER-READINESS（HANDOFF 下一优先级 4）· M5-PHASEB-100TURN-EVIDENCE（HANDOFF/README/记忆规格引用的权威结果）· SQLITE-CURRENT-STATUS-AUDIT 与 SQLITE-M5-100-ENDURANCE（HANDOFF 下一优先级 1/2）· COT-THREE-ARM-80TURN-EVIDENCE（下一优先级 3）· CAMPAIGN-WORLDINFO-AND-CARD-SHELL-PLAN 与 TEST-CARD-SHELL-EVIDENCE（进行中）· CARD-STUDIO-FEASIBILITY（活跃分析）· ARCHITECTURE-REVIEW-2026-07-26（本次评审）

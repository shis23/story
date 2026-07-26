# ZCode Prompt: M5 + Phase B 100-Turn Real Evidence

Work only in:

```text
C:\tmp\storyforge-m5-endurance
branch codex/m5-phaseb-100turn-evidence
```

Before running Cargo commands, set:

```powershell
$env:CARGO_TARGET_DIR = 'C:\tmp\storyforge-parallel-target'
```

This target directory is shared by the parallel workstreams to control C-drive
usage. Wait on Cargo locks when necessary, do not run `cargo clean`, and avoid
full-workspace gates until this branch's final verification.

Read completely before editing:

1. `docs/workstreams/M5-PHASEB-100TURN-EVIDENCE-PLAN.md`
2. `docs/archive/2026-07-26-completed-workstreams/M5-PRODUCTION-EVIDENCE-DAY-RESULT.md`（2026-07-26 归档）
3. `docs/archive/2026-07-26-completed-workstreams/EVAL-M5-PHASEB-RESULT.md`（2026-07-26 归档）
4. `docs/MEMORY-CONTEXT-COMPILER-SPEC-2026-07-11.md`
5. `docs/archive/2026-07-26-completed-workstreams/TURN-LIFECYCLE-SERVICE-RESULT.md`（2026-07-26 归档）

Implement the entire vertical slice, not only a skeleton. Use TDD: add failing
deterministic tests for scheduling, checkpoint/resume, budget limits, evidence
redaction/completeness, and acceptance classification before implementation.

The target is a production-faithful, resumable 100-accepted-turn real-model run
covering Director, dynamic Subagents, Editor, Summarizer, PostProcessor,
CharacterExtractor, Meta bookends, all three ReasoningMode values, tool modes,
world-info routes, private knowledge, Chronicle tools, regeneration variants,
QualityGate/autofix, cache invalidation, epoch rollover, cancellation, timeout,
and retry behavior according to the PLAN matrix.

Real-model settings:

```text
LLM_BASE_URL=https://cli.2529985.xyz/v1
LLM_MODEL=grok-4.5
LLM_API_KEY must come from the process environment only
```

Never write or echo the key. Before real calls, run dry-run and deterministic
gates. Execute stages 3 -> 12 -> 30 -> 100 accepted turns. Use hard suite-wide
budgets and fail closed. The full stage may use at most 700 calls unless an
existing stricter limit applies. Preserve sanitized checkpoints so an
interruption resumes without replaying accepted turns.

Use production H/E defaults for the primary run. You may add a clearly labeled
harness-only low-threshold compression micro-profile to exercise A/B/C, but do
not change production `200/4`, `H_anchor`, or `E`.

Evidence must contain usage, latency, cache, role, mode, hashes, epoch/revision,
Turn/Attempt status, Quality/autofix, Chronicle ranges, and assertions, but no
raw prompts/story/private text. A requested full stage that stops early must
exit non-zero and be reported as Partial/Inconclusive.

Also complete a real Phase B paired A/B matrix with at least 12 shared-seed
scenarios and report leak, quality, autofix, continuity/agency, token, latency,
and failure metrics.

Do not edit `docs/HANDOFF.md` or `docs/RELEASE-CHECKLIST.md`. Do not touch
SQLite, GUI, Android, or release publishing. Do not push. Make logical commits,
keep the worktree clean, and finish with
`docs/workstreams/M5-PHASEB-100TURN-EVIDENCE-RESULT.md` containing exact commands,
call counts, evidence locations, failures, and an honest acceptance level.

When finished, report branch, HEAD, commits, files, deterministic gates, real
call count, accepted turns, evidence summary, remaining risks, and merge advice.

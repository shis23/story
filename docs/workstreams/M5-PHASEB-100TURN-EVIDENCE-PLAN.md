# M5 + Phase B 100-Turn Real Evidence Plan

> Branch: `codex/m5-phaseb-100turn-evidence`
> Worktree: `C:\tmp\storyforge-m5-endurance`
> Base: `b46ddc8`
> Status: implementation plan; real-model evidence is not yet collected.

## Objective

Close the remaining M5 and Phase B evidence gap with a production-faithful,
budgeted, resumable 100-accepted-turn endurance run. The run must exercise the
real writing pipeline and shared Turn lifecycle rather than synthetic drafts.

The result must distinguish:

- probe execution success;
- functional acceptance;
- cache/latency/token observations;
- narrative-quality evidence;
- production parameter calibration.

Only the first four may be upgraded by this work. Do not change or claim that
`200/4`, `H_anchor=5`, or `E=10` are calibrated unless the evidence explicitly
supports such a later decision.

## Real-Model Configuration

- Base URL: `https://cli.2529985.xyz/v1`
- Model: `grok-4.5`
- API key: read only from `LLM_API_KEY` in the process environment.
- Never write the key to source, documentation, prompts, JSONL, logs, command
  examples, screenshots, panic messages, or RESULT files.

## Execution Stages

1. Dry-run validation: zero model calls; validate fixture, output directory,
   budgets, schema, disk space, and secret guards.
2. Canary: 3 accepted turns, maximum 30 model calls.
3. Coverage checkpoint: 12 accepted turns, maximum 120 total model calls.
4. Stability checkpoint: 30 accepted turns, maximum 300 total model calls.
5. Full endurance: 100 accepted turns, hard maximum 700 total model calls.

Each stage must be resumable from a sanitized checkpoint. A later stage may
start only when the previous stage passes its invariants. Budget exhaustion,
timeout, fixture loss, missing usage, secret detection, or inconsistent Turn
state must fail closed and preserve the last valid checkpoint.

## Coverage Matrix

The 100 accepted turns must follow a deterministic schedule recorded in the
evidence manifest. It must cover:

- `Director` on every writing turn;
- one, two, and three dynamic `Subagent` tasks;
- protagonist, supporting, extra/temporary, same-name distinct instances;
- `Editor` on every writing turn;
- `Summarizer` and `PostProcessor` persistence on every accepted turn;
- `CharacterExtractor` once at setup using the real complex-card fixture;
- `Meta` health/explain/typed-patch cases before and after the endurance run;
- `ReasoningMode::{Disabled,Native,Prompted}` in recorded blocks;
- native tool mode and text-fallback tool mode where supported;
- constant/selective/both world-info routes;
- private-owner legal recall, non-owner leak attempts, narration leaks, and
  explicit `must_not_reveal` probes;
- Chronicle search/get usage and early-fact retrieval after multiple epochs;
- overall regenerate, Editor-only regenerate, and Subagent-only regenerate;
- one bounded QualityGate Editor autofix, plus a non-fixable failure case;
- controlled cancel, timeout, and retry cases outside the 100 accepted turns;
- cache-stable turns, hook/fingerprint invalidation, and epoch rollover.

Recommended scheduled perturbations:

- overall regenerate near turns 11, 37, and 73;
- Editor-only regenerate near turns 18, 54, and 90;
- Subagent-only regenerate near turns 25, 61, and 97;
- private-knowledge adversarial cases near turns 20, 50, and 80;
- early-fact probes sampled across turns 1-15 and queried after turns 35, 65,
  and 95.

The exact schedule may change if required by existing APIs, but every matrix
row must remain represented and the reason for any substitution must be in the
RESULT.

## Context and Compression Profiles

- The primary 100-turn run uses production `H_anchor=5` and `E=10`.
- Production `200/4` remains unchanged.
- A separate harness-only micro-profile may temporarily use a low compression
  threshold (for example 8/4) to force A -> B -> C behavior. It must be labeled
  non-production and must not mutate defaults or user configuration.
- A harness-only compact epoch profile may be used as an additional stress
  probe, but cannot replace the production-default 100-turn run.

## Evidence Schema

Write sanitized JSONL and a machine-readable manifest under an explicitly
configured ignored evidence directory. Record at minimum:

- commit, branch, model, endpoint host, stage, seed, accepted turn number;
- Agent role, streaming flag, tool mode, reasoning mode, retry/regenerate type;
- call index, hard budget, prompt/cached/completion tokens, latency;
- system/history/tail hashes, request fingerprint, epoch id, membership counts;
- Turn/Attempt ids as non-secret stable ids, revisions, draft hash, terminal
  status, derivation status, QualityGate/autofix outcome;
- Chronicle A/B/C codes and source turn ranges without raw story text;
- early-fact probe ids and pass/fail results without sensitive content;
- suite assertions and final acceptance classification.

No raw prompt, story text, private knowledge, API response body, or full error
payload may be written to evidence.

## Phase B A/B

Add a fixed-fixture, shared-seed matrix comparing Phase B disabled/enabled on
at least 12 paired scenarios. Measure:

- explicit private leak rate;
- QualityGate Error/warning counts;
- autofix trigger and success rate;
- continuity/agency markers;
- prompt/cached/completion tokens;
- latency and total call count;
- output emptiness/truncation/failure rate.

Do not use the same model output as both subject and semantic judge without
labeling the limitation. Deterministic checks remain authoritative for secrets,
ids, polarity markers, causal markers, and structured state.

## Required Implementation

- Extend the existing real-eval harness rather than creating a parallel fake
  pipeline.
- Route Accept through the shared `TurnLifecycleService` production path.
- Add checkpoint/resume and stage manifests.
- Enforce suite-wide max calls, per-call timeout, hard deadline, and evidence
  size budget.
- Make partial/inconclusive results exit non-zero for the requested full stage.
- Keep all real tests ignored unless the explicit real-eval switch is present.
- Add deterministic tests for scheduling, budget boundaries, resume idempotency,
  redaction, evidence completeness, and failure classification.

## Validation

Minimum local gates:

```powershell
cargo fmt --all -- --check
cargo test -p harness-real-llm
cargo clippy -p harness-real-llm -p storyforge-infra-llm -p storyforge --all-targets -- -D warnings
powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\run-real-llm-smoke.ps1 -Suite eval -DryRun
```

Run real stages only after deterministic gates pass. Do not run unrelated GUI,
Android, SQLite cutover, or release-publish operations.

## Deliverables

- implementation and deterministic tests;
- resumable 100-turn real-eval runner;
- sanitized JSONL/manifest schema and evidence writer;
- actual evidence artifacts in an ignored directory;
- `docs/workstreams/M5-PHASEB-100TURN-EVIDENCE-RESULT.md`;
- logical commits and a clean worktree.

## Acceptance

The full stage passes only when all 100 intended turns reach an accepted
terminal state, revisions are monotonic, checkpoints resume idempotently,
Summarizer/PostProcessor persistence meets the declared threshold, no secret
or private-knowledge invariant fails, early facts remain reachable, and the
evidence manifest is complete.

If the endpoint, quota, or environment prevents 100 accepted turns, report the
highest completed checkpoint as `Partial Evidence`; do not call M5 complete.

## Prohibited Changes

- no production default tuning;
- no API key in the repository;
- no edits to `docs/HANDOFF.md` or `docs/RELEASE-CHECKLIST.md`;
- no SQLite/backend work;
- no GUI or Android claims;
- no push, rebase, force push, or modification of `main`.

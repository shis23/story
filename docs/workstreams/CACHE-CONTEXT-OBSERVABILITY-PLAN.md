# Cache and Context Observability Plan

- Branch: `codex/cache-context-observability`
- Baseline: `main@c3a972d`
- Worktree: `C:\tmp\storyforge-cache-context`
- Limited real-provider validation is authorized; production defaults remain unchanged

## Objective

Complete deterministic observability and regression coverage for prompt segmentation, provider
usage parsing, cache-prefix stability and long-session Context compilation. Produce evidence
that can later make a real-provider run interpretable without changing `200/4/H/E`.

## Required scope

1. Build a table-driven usage parser matrix for OpenAI/DeepSeek/Anthropic-compatible streaming
   and non-streaming schemas, final/partial chunks, missing usage and malformed values.
2. Record trustworthy agent role, streaming mode, system/history/tail hashes, request fingerprint,
   longest common message prefix and reusable-token estimate without prompt text.
3. Detect boot/setup samples separately from turn samples and prevent cross-turn attribution.
4. Add deterministic simulated-cache tests proving stable prefixes within an epoch and expected
   invalidation at hook changes and epoch rollover.
5. Run a deterministic 100+ accepted-turn Context compilation benchmark using synthetic drafts
   and Chronicle entries; record token estimates, near/band/overview membership, latency and
   serialized evidence size.
6. Add budget assertions for prompt growth slope, maximum near window and evidence redaction.
7. Produce a small machine-readable report and human RESULT; do not claim provider cache hits.

## Limited real-provider authorization

This workstream is the **only** one of the six parallel branches authorized to spend the
limited real-LLM quota.

- Endpoint: `https://cli.2529985.xyz/v1`
- Model: `grok-4.5`
- Credential source: shell environment variable `LLM_API_KEY` only
- Initial hard budget: at most 32 provider calls and 6 writing turns for this branch
- Retry policy: at most one retry for a retryable transport/server failure; no blind rerun loop
- Stop conditions: budget exhaustion, first unexplained schema/redaction failure, or evidence I/O
  failure must stop the real run

Run deterministic tests first. Real-provider calls are only for validating the observed usage
schema, streaming cache signal and segment-hash evidence after all local gates are green. They
must not be used to tune or change production `200/4/H/E` defaults.

The API key must never be written to a tracked or untracked project document, command transcript,
RESULT, JSONL evidence, test fixture, process argument, screenshot or Git commit. Do not print
environment variables. Reports may record only the endpoint host, model, call/token counts,
latency, cache usage, hashes and redacted outcomes.

## TDD and acceptance

- Parser fixtures first, including every previously observed cache-usage schema.
- No `0 >= 0` or boot-sample cache success conditions.
- Evidence writer must reject credentials, full messages, full responses and secret probes.
- Benchmark must be deterministic, bounded and runnable without network access.

## Boundaries

- No real LLM calls outside the limited authorization above, and no parameter tuning.
- Do not modify production `overview_max_entries`, compression `200/4`, `H_anchor` or `E`.
- Do not change Turn lifecycle, SQLite, plugin or release behavior.
- Do not edit `docs/HANDOFF.md`.

## Scoped gates

- infra-llm, domain/context and harness observability tests
- deterministic benchmark/report generation
- strict Clippy and fmt for affected crates
- `git diff --check c3a972d..HEAD`

## Delivery

Commit coherent steps, do not push, and create
`docs/workstreams/CACHE-CONTEXT-OBSERVABILITY-RESULT.md` with schema coverage, benchmark data,
redaction evidence, remaining real-provider questions and unchanged-default proof.

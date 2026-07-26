# Plugin Runtime Compatibility Follow-Up Plan

> Branch: `codex/plugin-runtime-followup`
> Worktree: `C:\tmp\storyforge-plugin-runtime`
> Base: `b46ddc8`

## Objective

Turn the current compatibility matrix and degraded shims into a broader,
executable plugin runtime contract. Improve event fidelity, cancellation,
permissions, persistence adapters, and audit inspection without claiming full
ST 99 compatibility or requiring GUI/manual testing.

This line must stay out of `tauri-app` storage code so it can run safely beside
the SQLite workstream.

## Scope

### Event and Command Semantics

- Audit every currently classified supported/degraded/unsupported event row.
- Close the `Committed` alias/state-change ambiguity with executable tests.
- Expand Slash/TavernHelper command-pipeline behavior, including unknown,
  partial, async, cancellation, and multi-command cases.
- Ensure unsupported single commands return explicit structured results and
  unsupported pipeline steps stop the pipeline.
- Add correlation/generation ids to all asynchronous plugin operations.

### Prompt Hook Runtime

- Exercise multiple plugins with ordered mutation, timeout, cancellation,
  duplicate/late response, plugin unload, and permission revocation.
- Add bounded concurrency and per-plugin timing/size budgets.
- Preserve fail-open/fail-closed behavior according to operation type and make
  the classification machine-readable.
- Verify final messages-level hook behavior for streaming and non-streaming
  generation without retaining raw prompts.

### Persistence and Degraded Helpers

- Replace ambiguous degraded shims with explicit adapter contracts.
- Implement a deterministic host-side persistence adapter for `saveChat` that
  can be injected/tested without editing `tauri-app` or relying on GUI.
- Specify popup/request-header behavior, permission checks, cancellation, and
  unsupported responses.
- Never expose authorization, API keys, prompt bodies, or raw error stacks to
  plugins lacking the relevant permission.

### Audit Query and Export

- Add query/filter helpers for plugin id, event, generation, outcome, time,
  duration, and correlation id.
- Add pagination/bounds and deterministic ordering.
- Re-sanitize records at query/export boundaries.
- Add tamper-evident record hashes or chain metadata without storing secrets.
- Add retention/size limits and hostile-record tests.
- Provide component-level audit inspection/export tests; no GUI acceptance
  claim is allowed.

### Compatibility Inventory

- Expand the executable matrix toward the ST 99 inventory with an explicit
  reason and fallback for every non-supported row.
- Generate a machine-readable compatibility report and Markdown summary.
- Keep long-tail behavior honest: supported, degraded, noop, and unsupported
  must remain distinct.

## Tests

- Node table-driven matrix tests;
- composable/App integration tests for cancellation and generation isolation;
- hostile audit record and permission tests;
- Rust PluginHost correlation/permission tests;
- deterministic persistence adapter tests;
- property tests for event aliases and export redaction.

## Validation

```powershell
Set-Location frontend
npm.cmd test
npm.cmd run build
Set-Location ..
cargo test -p storyforge-infra-plugin-host
cargo clippy -p storyforge-infra-plugin-host --all-targets -- -D warnings
git diff --check
```

## Deliverables

- expanded executable compatibility matrix;
- hardened prompt-hook runtime and cancellation contracts;
- injectable persistence/degraded-helper adapters;
- queryable, bounded, re-sanitized audit export;
- generated compatibility report;
- `docs/workstreams/PLUGIN-RUNTIME-FOLLOWUP-RESULT.md`;
- logical commits and a clean worktree.

## Prohibited Changes

- do not edit `crates/tauri-app` or SQLite/storage files;
- do not claim full ST 99 or real iframe/GUI acceptance;
- do not use real LLM calls;
- do not edit `docs/HANDOFF.md` or `docs/RELEASE-CHECKLIST.md`;
- do not push, rebase, force push, or modify `main`.

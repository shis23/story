# ZCode Prompt: Plugin Runtime Compatibility Follow-Up

Work only in:

```text
C:\tmp\storyforge-plugin-runtime
branch codex/plugin-runtime-followup
```

Before Rust gates, set `$env:CARGO_TARGET_DIR` to
`C:\tmp\storyforge-parallel-target`. Do not run `cargo clean`; wait for shared
Cargo locks and keep Rust validation scoped to the plugin-host crate.

Read:

1. `docs/workstreams/PLUGIN-RUNTIME-FOLLOWUP-PLAN.md`
2. `docs/workstreams/PLUGIN-COMPAT-MATRIX-RESULT.md`
3. current frontend plugin bridge/hooks/audit/composables and
   `crates/infra-plugin-host`.

Complete the full PLAN using TDD. Expand the executable compatibility inventory,
close the `Committed` alias ambiguity, harden Slash/TavernHelper pipelines,
exercise multi-plugin prompt hooks under timeout/cancel/unload/late-response
conditions, and add generation/correlation isolation.

Replace ambiguous degraded behavior with explicit adapter contracts. Implement
an injectable deterministic `saveChat` persistence adapter and specified
popup/request-header permission behavior without editing `tauri-app`.

Build query/filter/pagination/retention for audit records. Re-sanitize at every
query/export boundary and add hostile-record tests. Add tamper-evident hashes or
chain metadata without retaining raw prompts, messages, credentials, or stacks.

Generate machine-readable and Markdown compatibility reports. Keep supported,
degraded, noop, and unsupported classifications distinct. Do not claim full
ST 99 or real iframe/GUI acceptance.

Do not edit `crates/tauri-app`, SQLite/storage files, `docs/HANDOFF.md`, or
`docs/RELEASE-CHECKLIST.md`. Do not call real LLMs and do not push.

Make logical commits, keep the worktree clean, run all PLAN gates, and finish
with `docs/workstreams/PLUGIN-RUNTIME-FOLLOWUP-RESULT.md`. Report branch, HEAD,
commits, matrix counts, tests, compatibility changes, remaining degraded rows,
risks, and merge advice.

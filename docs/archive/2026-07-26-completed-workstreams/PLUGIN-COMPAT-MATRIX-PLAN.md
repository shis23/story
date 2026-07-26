# Plugin Compatibility Matrix Plan

- Branch: `codex/plugin-compat-matrix`
- Baseline: `main@c3a972d`
- Worktree: `C:\tmp\storyforge-plugin-compat`
- Deterministic only; no real GUI or LLM

## Objective

Turn the current plugin/ST compatibility claims into an executable deterministic matrix for
events, Slash commands, TavernHelper-style shims, prompt hooks, permissions, audit and
redaction. Fix concrete compatibility or safety gaps found by the matrix.

## Required scope

1. Inventory supported ST event names/aliases and map the ST 99-event surface to implemented,
   intentionally unsupported or no-op categories.
2. Test Slash registration/unregistration, aliases, argument parsing, pipe chaining, errors and
   collision behavior.
3. Test TavernHelper/common shim semantics currently promised by the project; unsupported calls
   must fail or degrade visibly instead of silently succeeding incorrectly.
4. Test prompt-hook ordering, timeout, cancellation, fail-open policy, role permissions and
   final-message mutation boundaries.
5. Test event subscription permissions and memory/body redaction for plugins lacking access.
6. Expand audit ring/export structures so tests can prove hook identity, timing, outcome and
   redaction without storing prompt bodies or secrets.
7. Add frontend PluginHost/usePluginBridge deterministic tests for mount/unmount, hidden hook
   hosts, status slots and message correlation. Browser IPC mocks remain labelled mock UI.

## TDD and acceptance

- Build a table-driven matrix before fixes.
- Every supported entry needs a passing test; every unsupported entry needs explicit behavior.
- Timeouts, malformed plugin messages and duplicate request IDs must not deadlock writing.
- Audit/export tests must prove absence of API keys, private memory and full prompt bodies.

## Boundaries

- No Tauri desktop interaction and no paid model calls.
- Avoid Turn lifecycle implementation files; use plugin-specific modules/adapters.
- Do not modify SQLite, import/export or Android build configuration.
- Do not edit `docs/HANDOFF.md`.

## Scoped gates

- plugin/domain/app-plugin-related Rust tests and strict Clippy
- frontend Node/component tests for PluginHost/usePluginBridge
- optional browser IPC mock smoke, explicitly labelled mock
- `git diff --check c3a972d..HEAD`

## Delivery

Commit coherent steps, do not push, and create
`docs/workstreams/PLUGIN-COMPAT-MATRIX-RESULT.md` containing the supported/unsupported matrix,
fixes, security evidence, tests and remaining real-plugin/GUI validation.

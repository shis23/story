# Gitea Actions is decommissioned

> Status: **DECOMMISSIONED - the `.gitea/workflows/*` files are no longer part of the release path.**
> Effective: 2026-09-06 (user decision; see the remote-CI row of `docs/RELEASE-STATUS.md`)
> Recorded by: review-goals (task-17, 2026-09-13)

## Conclusion

The three files under `.gitea/workflows/` are retained as historical records and are no
longer executed by any CI server:

| File | Historical purpose | Status |
| --- | --- | --- |
| `ci-gates.yml` | Deterministic gates (frontend + backend) on Gitea Actions | Decommissioned |
| `windows-gates.yml` | Gates on the Gitea self-hosted Windows runner | Decommissioned |
| `release-host-evidence.yml` | Host-side release evidence package (manual dispatch) | Decommissioned; its package format and verifier are still reused by local scripts |

What was stopped (per `docs/RELEASE-STATUS.md`): the act_runner and its running task
containers were stopped; queued runs 34-36 will not execute; the repository
`has_actions` flag was turned off. Reason: runner capacity plus a workspace-cache
non-fast-forward failure (the root cause of the all-red run #33).

## The release path now

- Remote builds and publishing run on **GitHub Actions**: `.github/workflows/release.yml`
  (jobs `windows` / `android` / `release`).
- The v0.1.2 tag build is evidenced by run `34013739416` (success).
- The GitHub jobs are build-oriented: `npm ci` / `npm run build`, one frontend contract
  test (`node --test tests/tauri-command-contract.test.mjs`), `cargo tauri build --ci`,
  artifact collection/upload and the release publish step. They do **not** run the
  11-step deterministic gate (no `cargo fmt` / `clippy` / `cargo test` / Pester); that
  gate is executed locally by `scripts/verify-release.ps1`.

## What this means for the local tests (important)

- The assertions over `.gitea/workflows/*` in `scripts/tests/ReleaseBuild.CI.Tests.ps1`
  and `scripts/tests/ReleaseBuild.RunnerReadiness.Tests.ps1` are a **historical
  contract**: they only keep the retained files from silently rotting. They do **not**
  certify that any CI provider is active.
- The same suite now also asserts that the release entry scripts never depend on
  `.gitea`, and that `release.yml` remains the release path with platform-suffixed
  checksum files - a guard against reverting to the pre-decommission state.
- Therefore a green run of those tests **must not** be reported as "CI gates passed";
  the gate status lives in the 11-step records of `docs/RELEASE-STATUS.md`.

## Before re-enabling Gitea Actions

Restoring the server side (`act_runner`, `has_actions`) requires a new user decision.
Until then, do not wire these workflows back into the release path.

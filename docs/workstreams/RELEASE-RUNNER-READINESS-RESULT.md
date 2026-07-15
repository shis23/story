# Release Runner Readiness Result

> Branch: `codex/release-runner-readiness`
> Worktree: `C:\tmp\storyforge-release-runner-readiness`
> Base: `b54612c` (plan commit on this branch; merge-base with main is `8732e21`)
> Date: 2026-07-15

## Objective

Raise Gitea/host release runners to a state where operators can preflight
locally, verify evidence packages offline, and keep workflow static contracts
fail closed — **without** deploying or registering a remote runner, signing,
publishing, or claiming GUI/device/remote CI success.

## Summary

Added:

1. **Local preflight** (`Test-ReleaseRunnerPreflight` +
   `scripts/test-release-runner-preflight.ps1`) with machine-readable reports
   that distinguish host-only readiness, missing dependencies, and explicit
   bundle/APK authorization. GUI, device, and remote CI remain
   `not_claimable`.
2. **Offline evidence verifier** (`Test-ReleaseEvidencePackage` /
   `Assert-ReleaseEvidencePackage` + `scripts/verify-release-evidence.ps1`)
   that rejects missing subjects/sidecars, hash mismatches, UTF-8 BOM
   sidecars, path escapes, schema drift, sensitive notes/warnings, and
   missing inventory/provenance for `build_status=ok`. Dry-run packages need
   `-AllowDryRun` and never claim remote CI.
3. **Workflow static contract** (`Assert-ReleaseWorkflowStaticContract`) for
   pinned actions, `npm ci`, secret scan, retention-days 14, host-only
   defaults, and real YAML parser requirements.
4. **Credential-free ops doc** at
   `docs/operations/gitea-host-release-runner.md` (labels, least privilege,
   cache, retention, concurrency, manual `workflow_dispatch`, no tokens).
5. **Pester suite** `scripts/tests/ReleaseBuild.RunnerReadiness.Tests.ps1`
   (27 tests) registered in `run-release-build-tests.ps1`.

Also fixed three **test fixtures** that embedded static `sk-…` strings and
blocked the repository-wide fail-closed secret scan. Fixtures now assemble
the secret shape at runtime. This is not product business logic and does not
weaken the scanner.

**This line proves local readiness and offline verification only.** It does
**not** prove remote Gitea runner execution, Tauri bundle success on CI,
signed APKs, GUI acceptance, or device acceptance.

## Files Changed

| File | Change |
|------|--------|
| `scripts/release-build/ReleaseBuild.Common.ps1` | Preflight, offline verifier, workflow static contract helpers |
| `scripts/test-release-runner-preflight.ps1` | **New** CLI for local preflight reports |
| `scripts/verify-release-evidence.ps1` | **New** CLI for offline evidence package verification |
| `scripts/tests/ReleaseBuild.RunnerReadiness.Tests.ps1` | **New** 27 Pester tests (RED→GREEN) |
| `scripts/tests/run-release-build-tests.ps1` | Registers readiness suite |
| `docs/operations/gitea-host-release-runner.md` | **New** credential-free runner ops guide |
| `docs/workstreams/RELEASE-RUNNER-READINESS-RESULT.md` | **New** this document |
| `crates/harness-real-llm/src/endurance.rs` | Runtime-assembled secret-shaped test fixture |
| `crates/infra-sqlite/tests/cutover.rs` | Runtime-assembled secret-shaped test fixture |
| `crates/infra-sqlite/tests/reverse_export.rs` | Runtime-assembled secret-shaped test fixture |

## Explicit non-claims

| Claim | Status |
|-------|--------|
| Local preflight for host-only Windows/Android/CI profiles | Proven locally |
| Offline re-hash / package validation (incl. dry-run with `-AllowDryRun`) | Proven locally |
| Workflow static governance (pins, npm ci, retention, host-only) | Proven locally via Pester + real PyYAML |
| Remote Gitea Actions job execution | **Unverified** |
| Bundle / APK / signing / publish | **Not performed** |
| GUI / Android device acceptance | **not_claimable** |

## Verification (raw)

### Full release Pester suite

```text
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/tests/run-release-build-tests.ps1
```

Result: **PASS** (exit 0)

| Suite | Passed | Failed |
|-------|--------|--------|
| `ReleaseBuild.Tests.ps1` | 37 | 0 |
| `ReleaseBuild.Pipeline.Tests.ps1` | 14 | 0 |
| `ReleaseBuild.CI.Tests.ps1` | 31 | 0 |
| `ReleaseBuild.RunnerReadiness.Tests.ps1` | 27 | 0 |
| **Total** | **109** | **0** |

Closing line from runner:

```text
Release build tests passed.
```

### Dry-run host pipelines

```text
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/run-release-build.ps1 -DryRun
```

Exit **0**. Key lines:

```text
Windows release build finished with status=dry-run.
Reminder: host build success is not GUI/device PASS.
```

```text
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/run-android-host-pipeline.ps1 -DryRun
```

Exit **0**. Key lines:

```text
Android host pipeline finished with status=dry-run.
Reminder: host smoke/APK build is not device PASS.
```

### Preflight + offline verifier

```text
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/test-release-runner-preflight.ps1 -Profile windows-host
```

Exit **0**. Claims block includes `gui/android_device/remote_ci = not_claimable`.

```text
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/verify-release-evidence.ps1 `
  -EvidenceDir artifacts/release-build/windows-<run> -AllowDryRun
```

Exit **0**. Notes include “Accepted as local dry-run evidence only; not remote CI”.

### Other gates

```text
git diff --check
```

Exit **0** after EOF whitespace trim.

Secret scan:

```text
OK: secret scan found no matches in Git-tracked or untracked build-input files.
```

## Fail-closed coverage added

| Failure mode | Behavior |
|--------------|----------|
| Missing cargo/rustc/npm/node (host) | Preflight `missing_dependencies`, `-FailClosed` throws |
| Missing python/PyYAML (ci-gates) | Preflight fails closed |
| Bundle without cargo-tauri | `needs_explicit_authorization` / fail closed |
| APK without ANDROID_HOME/NDK_HOME | missing + fail closed |
| Missing subject / sidecar | Verifier rejects |
| Hash mismatch | Verifier rejects |
| UTF-8 BOM sidecar | Verifier rejects |
| Path escape (`..` / absolute) | Verifier rejects |
| Unknown schema version | Verifier rejects |
| Secret-like notes/warnings | Verifier rejects (values redacted) |
| Missing inventory/provenance for `ok` | Verifier rejects |
| Dry-run without `-AllowDryRun` | Verifier rejects |
| Workflow without real YAML parser | Static contract / existing syntax gate fail closed |

## Follow-up hardening (adversarial verifier pass)

Additional fail-closed verifier fixes landed after the initial readiness commit:

1. Subjects, sidecars, inventory, and parent path segments reject
   junction/symlink/reparse points; canonical path escape is rejected.
2. Missing `EvidenceDir` and all verifier errors are path/secret redacted
   (no raw host home path, no secret-shaped fragments).
3. Provenance notes use the generic `Find-ReleaseSecretPatternFindings` scanner
   (api_key / Bearer / authorization / sk-* etc.), not sk-* alone.
4. `acceptance.remote_ci` is validated; `claimed`/`passed` fail closed and
   `remote_ci_claim` returns a controlled form (no fixed `false` cover-up;
   secret-shaped claims are redacted).
5. Manifest vs provenance `commit` / `branch` / `target` identity must match.
6. `partial` / `failed` packages fail closed; CLI never prints
   `VERIFICATION PASSED` or exits 0 for those statuses.
7. Real-process CLI tests cover missing dir redaction and partial/failed exits.

### P0 subject exact-set binding + identity/scanner tightening

Further P0 hardening:

1. For `build_status=ok`, `manifest.staged_subjects` (present) and
   `provenance.subjects` form a **bidirectional exact-set** keyed by
   normalized `relative_path|kind|status|sha256|size_bytes`. Duplicates, missing
   members, extra members, and field disagreements fail closed.
2. Every present **staged subject** is fully checked (exists, non-reparse,
   sidecar, rehash, size) even when provenance subjects exist — provenance
   presence never skips staged-subject verification.
3. Attack regression: staged `claimed.exe` while provenance only lists
   `checked.exe` fails closed.
4. `commit` / `branch` / `target` must be non-empty; `commit` must match strict
   git SHA (`^[0-9a-fA-F]{7,40}$`) on both manifest and provenance.
5. Generic secret scanner recursively walks all string leaves in
   manifest/provenance graphs and covers bare `Bearer`, unquoted
   `api-key`/`token`/`credential`, plus existing patterns.
6. TOCTOU trust model is explicit via
   `Get-ReleaseEvidenceVerifierTrustModel` (`open-then-hash` with reparse
   rejection; package immutability assumed for the verification window).

### P0 source vs staged subject path domains + P1 scanner/size hardening

1. **Path-domain split:** `manifest.artifacts[*].relative_path` keeps the
   runner **source** tree path (e.g. `target/release/storyforge.exe` or APK
   build outputs). Offline verification never treats those source paths as
   subject files.
2. **Single staged subject record:**
   `New-ReleaseStagedSubjectRecord` /
   `manifest.staged_subjects[]` with
   `relative_path` under `subjects/…`, optional
   `source_relative_path`, `size_bytes`, `sha256`, `kind`, `status`,
   `hash_sidecar`. Windows and Android runners both emit this structure after
   staging.
3. **Binding surface:** for `build_status=ok`, exact-set bind is
   `manifest.staged_subjects` ↔ `provenance.subjects` on the staged domain only.
   Source paths are not forced equal to `subjects/…`.
4. **Topology fixtures:** readiness suite includes real runner topology
   packages (source ≠ staged) and rejects packages that only present
   source-relative paths for offline checks.
5. **P1 size_bytes:** required, non-negative, and rechecked against the staged
   file length on every present staged/provenance subject.
6. **P1 scanner:** recursive walk scans object **keys** and values; depth
   overflow beyond max depth **fail-closes** (no silent truncation).
7. TOCTOU model unchanged: open-then-hash + reparse rejection; package
   immutability assumed during verification (not a sealed OS snapshot handle).

### P0 runner-shape flatten + P1 source mapping / sidecar grammar

1. **Runner shape:** Windows/Android runners no longer use
   `@(Copy-ReleaseEvidenceSubjects ...)`. They assign directly and flatten via
   `ConvertTo-ReleaseStagedSubjectArray` so unary-comma `object[]` returns do
   not nest into `manifest.staged_subjects`.
2. **Regression:** nested `@(Copy-...)` input flattens to flat staged records;
   non-dry-run `build_status=ok` package verifies offline.
3. **P1 source mapping:** each present `staged_subject` must uniquely map to
   one present `manifest.artifact` by
   `source_relative_path` + `kind` + `sha256` + `size_bytes` + `status`.
   Missing source, orphan source, field inconsistency, and duplicate mapping
   fail closed. Offline verifier does **not** reopen source files outside the
   package; it checks the declared chain only.
4. **P1 sidecar grammar:** sidecars must be a **single** standard
   `sha256sum` line (`^[0-9a-f]{64} \*<basename>$`). Hash-only, multi-line, and
   path-bearing basenames fail closed.

Full suite after this pass: **136** Pester tests passed
(37 + 14 + 31 + 54). Windows/Android dry-runs exit 0. `git diff --check` clean.

## Risks / follow-ups

1. **Remote runner still unverified.** Operators must register a runner with
   their own short-lived Gitea token (never commit it) and trigger
   `workflow_dispatch`.
2. Bundle/APK evidence still requires explicit authorization and host tools;
   default workflow path remains host-only.
3. Workflow static contract checks YAML/governance text contracts; it does not
   emulate Gitea Actions runtime semantics.
4. Fixture string assembly is only for static scanners; runtime secret-shape
   rejection tests remain intact.
5. Creating symlinks/junctions in adversarial tests requires local filesystem
   privilege; if mklink fails the suite surfaces that as a hard error.

## Recommendation

**Conditional yes** for local release-runner readiness and offline verification.
Do **not** treat this branch as remote CI green, bundle/APK proven, signed, or
GUI/device accepted.

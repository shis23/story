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
   registered in `run-release-build-tests.ps1`. The initial readiness slice had
   27 cases; the current adversarial suite contains 97 cases (see the latest
   validation status below rather than treating the initial count as current).

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
| `scripts/tests/ReleaseBuild.RunnerReadiness.Tests.ps1` | Initially 27 new Pester tests (RED→GREEN); current suite has 97 cases |
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
| Workflow static governance (pins, npm ci, retention, host-only) | Historical pre-follow-up evidence at `45d84dd`; latest hardening still requires a writable host with a real YAML parser |
| Remote Gitea Actions job execution | **Unverified** |
| Bundle / APK / signing / publish | **Not performed** |
| GUI / Android device acceptance | **not_claimable** |

## Verification (raw, historical initial delivery)

### Full release Pester suite

```text
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/tests/run-release-build-tests.ps1
```

Historical result: **PASS** (exit 0). The current follow-up validation status
appears in [Latest local verification status](#latest-local-verification-status)
and supersedes this table for merge readiness.

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

### P0 reverse artifact coverage + full workflow offline verifier

1. **Reverse exact-set:** every present `manifest.artifact` must be uniquely
   covered by a staged subject (`source_relative_path` mapping). Adding an
   uncovered present artifact (e.g. `target/release/uncovered.exe`) fails closed
   even if staged↔provenance already match.
2. **Remote workflow gate:** both `windows-host-evidence` and
   `android-host-evidence` jobs call `Assert-ReleaseEvidencePackage` before
   upload. Weak schema/manual rehash-only loops are removed so mapping, sidecar
   grammar, reparse rejection, and recursive secret scan cannot be bypassed.
3. **Static contract (controlled verifier + upload step metadata):**
   `full_offline_verifier` is decided by real YAML parse
   (`Test-ReleaseHostEvidenceVerifierOrder` via PyYAML or Node yaml/js-yaml)
   that extracts full step metadata (`shell`, `continue-on-error`, `if`,
   `with.path`, `run`), then PowerShell AST + binding checks.
   Each of `windows-host-evidence` / `android-host-evidence` must have a
   controlled verifier `run` step **before** `actions/upload-artifact` with:
   - `shell: pwsh` (or `powershell`)
   - no `continue-on-error: true`
   - no upload `if: always()`
   - flat top-level reachable `Assert-ReleaseEvidencePackage` CommandAst
   - required dot-source of `scripts/release-build/ReleaseBuild.Common.ps1`
   - no `-AllowDryRun`
   - `-EvidenceDir` and upload `with.path` both bound to
     `${{ steps.evidence.outputs.dir }}`
   The verifier step is modeled as a **restricted flat script**: before the
   wanted command is reached, only simple assignments and a single-command
   required dot-source are allowed (`Test-ReleaseRunInvokesCommand` +
   `Test-ReleaseVerifierStepScriptContract`).
   Rejected: comment-only, `Write-Host`/string decoys, assignment-only names,
   commands inside `if`/loop/function/try-catch/nested scriptblock, any
   pre-verifier control flow (including `if ($true) { return|exit|throw }`),
   `shell: bash`, `continue-on-error: true`, upload `if: always()`, forged
   common.ps1 source, path/EvidenceDir mismatch, `-AllowDryRun`, and calls
   after top-level `return`/`exit`/`throw`. Structural YAML fallback is never
   PASS. Production workflow text was not rewritten for this pass; it already
   matches the controlled step shape.
4. **Main integration + M5 fixture:** merged `main` (`12b5f44`) into this
   branch. `evidence_retention_deterministic.rs::seal_refuses_secret_payload`
   now runtime-assembles its secret-shaped payload so
   `Invoke-ReleaseSecretScan` passes without weakening `seal_run` /
   `ForbiddenPayload` rejection. Prior endurance/SQLite runtime-assembled
   fixtures remain intact.

At commit `45d84dd` (before the subsequent static-contract hardening below),
the full suite recorded **150** Pester tests passed (37 + 14 + 31 + 68), with
Windows/Android dry-runs exiting 0.
`Invoke-ReleaseSecretScan` OK. `git diff --check main..HEAD` clean.

**Still not proven:** remote Gitea runner execution, GUI acceptance, device
acceptance, signing/publish.

## Latest adversarial static-contract hardening (post-`45d84dd`)

This follow-up closes additional fail-open routes found by independent review.

1. **Fresh producer/output binding:** each host build now owns `id: evidence`,
   creates a UUID-named controlled `-OutputDir`, verifies its own
   `manifest.json`, and writes that exact directory to `$GITHUB_OUTPUT`.
   The workflow no longer selects a `windows-*` / `android-*` directory by
   latest timestamp, so residual self-hosted-runner evidence cannot be picked
   up after a skipped or unrelated build.
2. **Producer contract:** static validation now uses a restricted producer AST,
   not merely a matching command. It permits only the fixed GUID-namespaced
   `evidenceDir` assignment, read-only path checks, the controlled `pwsh -File`
   build call(s), and one exact `dir=$evidenceDir` write to `GITHUB_OUTPUT`.
   It also requires a fresh-directory guard, native `$LASTEXITCODE` guard, and
   `manifest.json` guard before the output is published. The Windows-only
   `skip_bundle` value is bound as an inert step environment value, exact-
   allowlisted to empty/`true`/`false`, then branched on; it is no longer
   interpolated into PowerShell source. The contract rejects producer-side
   helper rewrites, stale/rebound output paths, `exit`/catch masking, arbitrary
   commands, non-`pwsh` shells, malformed dispatch input, and false-valued
   `-SkipBundle` switches. Its `id: evidence` must feed both verifier and upload.
3. **Executable CI gates:** the required `frontend-gate` `npm ci` and
   `secret-scan` command are each a single flat, unconditional command after an
   exact checkout in the intended job and working directory. `exit 0`,
   `Set-Location`, command/here-string decoys, fake jobs, and tolerated or
   conditional steps cannot satisfy either gate. PowerShell launchers use a
   closed `-NoProfile -File` grammar, so `-Command`, abbreviated `-Co`/`-Enc`,
   `-WorkingDirectory`, and `-SecretScanOnly:$false` do not count.
4. **Checkout integrity surface:** host and controlled CI checkouts are exact
   `actions/checkout@v4`; their `with` keys must use the exact lowercase
   `fetch-depth` spelling (if present), be unique, and otherwise fail closed.
   `github-server-url`, repository/ref/token/path, case variants, and sparse
   checkout-style inputs therefore fail closed.
5. **Workflow governance:** non-string action `uses`, job-level reusable
   workflow `uses`, and action references outside a fixed allowlist of
   version tags/labels fail closed. Root permissions other than exact lowercase
   `contents: read`, and any job-level permissions override, also fail closed.
   PyYAML/Node parsing rejects duplicate keys, anchors, aliases, and merge
   keys; controlled `if` and `continue-on-error` fields must be absent, avoiding
   YAML 1.1/1.2 coercion ambiguity. The allowlist is **not** an immutable
   commit-SHA supply-chain guarantee: a compromised or retagged upstream action
   remains outside the protection offered by this repository-side contract.
6. **Trigger continuity:** the raw workflow source must contain a root literal
   `on:` (or quoted `"on":`) key before parser-normalized metadata is trusted,
   and `ci-gates.yml` must contain both `push` and `pull_request`. A YAML 1.1
   parser's `True` key cannot masquerade as the trigger, so removing or
   replacing `on:` cannot leave a green static contract while making CI inert.
7. **Trusted execution topology:** governed host jobs now permit only their
   fixed setup run, controlled producer, verifier, and adjacent upload. The
   `frontend-gate` prefix is exactly checkout → setup-node → `npm ci`, and the
   `secret-scan` prefix is exactly checkout → scan. An intervening `run` step
   cannot rewrite a helper, `package-lock`, or scan target while leaving a
   command-shaped gate behind.
8. **Closed PowerShell program grammar:** `using module`, `#requires`, named
   blocks, and other preambles are rejected before restricted commands are
   evaluated. Producer helper invocations also use an exact post-`-File`
   argument set, so Android `-BuildApk`/`-DryRun` and other appended behavior
   flags cannot retain a host-only green claim.
9. **Execution-mode and context binding:** every controlled host step must be
   exactly one of `uses` or `run`; an ambiguous step carrying both cannot make a
   command-shaped verifier/gate look executed. Workflow/job default shells,
   containers, services, and uncontrolled environment overrides are rejected.
   The only host-step environment exception is the exact inert Windows
   `skip_bundle` binding described above; host job environment remains exactly
   `CARGO_TARGET_DIR=""` and `RUST_LOG="info"`.
10. **Parser readiness and parser parity:** `pester-release-tests` now installs
   and verifies `PyYAML==6.0.2` before parser-backed Pester contracts run.
   Both metadata adapters label mappings consistently, reject numeric
   anchor/alias names, and the Node path accepts only integral retention days.

### Latest local verification status

The Codex execution sandbox used for this follow-up has neither PyYAML nor Node
`yaml`/`js-yaml`, and it rejects creation beneath the default
`artifacts/release-build` root. The first condition correctly fail-closes
workflow validation; together they prevent a truthful all-green parser-backed
Pester or standalone default-output dry-run claim here.

Completed in this sandbox:

- PowerShell parser validation of changed helper and Pester source: PASS.
- Direct AST adversarial probes: legitimate `npm ci`, secret-scan, and
  producer invocations are recognized; here-string/`Write-Host` decoys,
  `-Command`/`-Co`/`-Enc` launchers, false-valued switches, `exit 0` masking,
  wrong working directories, stale-output rebinding, helper rewrites,
  `using module`/`#requires` preambles, and appended Android/Windows behavior
  flags are rejected.
- The three non-parser suites passed: **37 + 14 + 32**. The parser-dependent
  readiness suite ran **72/97**; its remaining **25** failures are all tests
  that intentionally require a real YAML parser and therefore fail closed in
  this sandbox. This is recorded as a blocked validation, not a passing suite.
- Existing Windows and Android dry-run Pester coverage passed in the pipeline
  suite using temporary output roots. Direct default-output dry-runs fail in
  this sandbox at directory creation before any build work; no new remote CI,
  GUI, device, signing, or publish claim follows.
- Both embedded Node YAML adapters pass `node --check`; their runtime use still
  requires the separately installed `yaml` or `js-yaml` package.
- `Invoke-ReleaseSecretScan`: PASS.
- `git diff --check`: PASS.

Required before changing the recommendation to final merge-ready: rerun all
four Pester files and Windows/Android dry-runs in a writable Windows host with
PyYAML (or Node `yaml`/`js-yaml`) available and verify that all four Pester
files are green. The current source contains 97 readiness Pester cases; no new
remote runner, GUI, device, signing, or publish claim is made.

The contract prevents a workflow-authored producer from rewriting the checked-in
verification helper before invoking it. It intentionally does **not** claim to
defend against a compromised runner profile, a compromised checkout/ref, a
malicious action, or a process outside the controlled workflow steps that can
modify the checkout on disk. Operators still need a protected ref and an
isolated/clean runner; those are runner trust-boundary controls, not something
a same-repository workflow can attest by static inspection alone. Likewise,
`permissions: contents: read` is statically declared and checked here; whether
the installed Gitea runner/version enforces it requires a remote preflight and
is not locally claimed.

## Risks / follow-ups

1. **Remote runner still unverified.** Operators must register a runner with
   their own short-lived Gitea token (never commit it) and trigger
   `workflow_dispatch`.
2. Bundle/APK evidence still requires explicit authorization and host tools;
   default workflow path remains host-only.
3. Workflow static contract checks YAML/governance text contracts; it does not
   emulate Gitea Actions runtime semantics.
4. Remote preflight should exercise tag/empty, `true`, `false`, and malformed
   manual `skip_bundle` input. The producer now rejects malformed values as
   inert data before branching, but this repository cannot attest the installed
   Gitea server/runner's workflow-dispatch and expression compatibility.
5. Fixture string assembly is only for static scanners; runtime secret-shape
   rejection tests remain intact.
6. Creating symlinks/junctions in adversarial tests requires local filesystem
   privilege; if mklink fails the suite surfaces that as a hard error.

## Recommendation

**Conditional yes** for local release-runner readiness and offline verification.
Do **not** treat this branch as remote CI green, bundle/APK proven, signed, or
GUI/device accepted.

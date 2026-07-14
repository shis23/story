# Release CI and Reproducible Evidence Result

> Branch: `codex/release-ci-evidence`
> Worktree: `C:\tmp\storyforge-release-ci`
> Base: `b46ddc8` (merge-base with `main`)
> Date: 2026-07-13

## Objective

Convert the host-side release runners into a repeatable Gitea Actions evidence
pipeline with bounded jobs, deterministic manifests, artifact retention, and
truthful Windows/Android host claims. This line does not perform publishing,
signing, GUI acceptance, or physical-device acceptance.

## Summary

Added two Gitea Actions workflows (fast PR gates + manual/tagged host evidence),
extended the production PowerShell helpers with provenance attestation, manifest
schema validation, hash sidecars, archive integrity verification, and workflow
YAML syntax validation. Wired these into both production runners (Windows host
and Android host). Added comprehensive Pester coverage for every new helper.

**Remote Gitea Actions execution is unverified** — no Gitea Actions runner is
available in this environment. All evidence below is from local validation only.

## Prior Implementation Commits

| # | Hash | Message |
|---|------|---------|
| 1 | `db53e6c` | `docs(workstream): plan release CI evidence` |
| 2 | `49f5cbf` | `feat(release): add provenance, manifest schema, hash, and workflow validation helpers` |
| 3 | `3dd022d` | `feat(release): wire provenance, hash sidecars, schema validation into runners` |
| 4 | `dcdb5cc` | `ci(gitea): add fast gate and host artifact evidence workflows` |
| 5 | `65dd82f` | `docs(workstream): chronicle release CI evidence result` |
| 6 | `ad9da1b` | `fix(release-ci): stage subjects, harden scan/YAML/workflows for offline verification` |
| 7 | `afae42a` | `fix(release-ci): pin tauri-cli path, require real YAML parser, fail-closed untracked scan` |
| 8 | `71db1a3` | `fix(release-ci): verify node yaml fallback and host excludes` |

## Files Changed

| File | Change |
|------|--------|
| `.gitea/workflows/ci-gates.yml` | **New** — fast PR/push gates: fmt, clippy, test, frontend, Pester, secret scan, workflow syntax |
| `.gitea/workflows/release-host-evidence.yml` | **New** — manual/tagged host artifact evidence (Windows + Android host) |
| `scripts/release-build/ReleaseBuild.Common.ps1` | **Modified** — added `New-ReleaseProvenance`, `Write-ReleaseHashFile`, `Test-ReleaseArchiveIntegrity`, `Assert-ReleaseManifestSchema`, `Test-ReleaseWorkflowSyntax` |
| `scripts/run-release-build.ps1` | **Modified** — wired provenance, hash sidecars, schema validation, retention empty-guard |
| `scripts/run-android-host-pipeline.ps1` | **Modified** — wired provenance, hash sidecars, archive integrity, schema validation, SBOM inventory, retention empty-guard |
| `scripts/tests/ReleaseBuild.CI.Tests.ps1` | **New** — 31 Pester tests for provenance, hash files, archive integrity, manifest schema, workflow validation, governance, Node-only parser dispatch, and global-excludes handling |
| `scripts/tests/run-release-build-tests.ps1` | **Modified** — registered `ReleaseBuild.CI.Tests.ps1` in the test runner |
| `docs/workstreams/RELEASE-CI-EVIDENCE-RESULT.md` | **New** — this document |

## Gitea Actions Workflows

### `ci-gates.yml` — Fast PR / Push Gates

Triggered on all pushes and pull requests. Seven parallel jobs:

| Job | Runner | Timeout | Purpose |
|-----|--------|---------|---------|
| `rust-fmt` | ubuntu-latest | 10m | `cargo fmt --all --check` |
| `rust-clippy` | ubuntu-latest | 30m | `cargo clippy --workspace --all-targets -- -D warnings` |
| `rust-test` | ubuntu-latest | 45m | `cargo test --workspace` |
| `frontend-gate` | ubuntu-latest | 20m | `npm ci` (strict), `npm test`, `npm run build` |
| `pester-release-tests` | windows-latest | 20m | Full Pester suite + `verify-release -SecretScanOnly` |
| `secret-scan` | windows-latest | 10m | Fail-closed repository secret scan |
| `workflow-syntax` | windows-latest | 10m | YAML syntax validation of all `.gitea/workflows/` files with pinned PyYAML |

**Governance features:**
- `concurrency: { group: ci-gates-${{ github.ref }}, cancel-in-progress: true }`
- `permissions: { contents: read }` (least-privilege)
- Pinned action versions: `actions/checkout@v4`, `actions/setup-node@v4`, `actions/upload-artifact@v4`, `dtolnay/rust-toolchain@stable`

### `release-host-evidence.yml` — Manual / Tagged Host Evidence

Triggered on `workflow_dispatch` (with optional `skip_bundle` input) and `push`
to tags matching `v*`. Two jobs:

| Job | Runner | Timeout | Purpose |
|-----|--------|---------|---------|
| `windows-host-evidence` | windows-latest | 60m | `run-release-build.ps1`, manifest schema validation, artifact upload |
| `android-host-evidence` | windows-latest | 45m | `run-android-host-pipeline.ps1` (host smoke, no APK), manifest schema validation, artifact upload |

**Governance features:**
- `concurrency: { group: release-host-evidence-${{ github.ref }}, cancel-in-progress: true }`
- `permissions: { contents: read }`
- `CARGO_TARGET_DIR: ''` env override prevents workstation path inheritance
- `retention-days: 14` on all uploaded artifacts
- Android job omits `-BuildApk` — fails closed if APK evidence is requested without SDK/NDK
- No `set-output` of secret values; no environment dumps

## New Helper Functions

All in `scripts/release-build/ReleaseBuild.Common.ps1`:

| Function | Purpose |
|----------|---------|
| `New-ReleaseProvenance` | Builds an unsigned host provenance attestation referencing artifacts by sha256 digest. Declares it is not SLSA/cosign/in-toto. |
| `Write-ReleaseHashFile` | Writes `<file>.sha256` sidecar in `sha256sum -c` format (`<hash> *<basename>`). |
| `Test-ReleaseArchiveIntegrity` | Opens a zip/APK archive with `System.IO.Compression` to confirm it is not truncated/corrupt. |
| `Assert-ReleaseManifestSchema` | Validates manifest required fields, `build_status` values, acceptance scope (GUI/device must be `not_claimed`), and that present artifacts always carry sha256. |
| `Test-ReleaseWorkflowSyntax` | Requires a real YAML parser: PyYAML, or Node `yaml`/`js-yaml` when installed. The production path fails closed if neither is available; returns `{ Valid, ErrorCount, Errors }`. |

## Local Validation Gates

### Pester Tests (82 total, 0 failed)

```
ReleaseBuild.Tests.ps1:           Passed: 37  Failed: 0
ReleaseBuild.Pipeline.Tests.ps1:  Passed: 14  Failed: 0
ReleaseBuild.CI.Tests.ps1:        Passed: 31  Failed: 0
```

Command:
```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\tests\run-release-build-tests.ps1
```

**Parser evidence boundary:** this workstation has neither PyYAML nor a global
Node `yaml`/`js-yaml` package, so generic local parser tests verify the required
`Engine=none` fail-closed result. The Node-only tests mock Python discovery,
inject an isolated `yaml.parse` adapter, and call the public dispatcher with
JSON (a YAML subset), proving Node selection plus the requested-workflow
argument for both valid and malformed input. This is a wrapper-contract test,
not a compatibility suite for a third-party YAML package. The actual workflow
syntax job installs pinned PyYAML before parsing every workflow, but that remote
job has not yet run.

### Dry Runs

| Script | Exit | Status | Provenance | Schema |
|--------|------|--------|------------|--------|
| `run-release-build.ps1 -DryRun` | 0 | `dry-run` | written | validated |
| `run-android-host-pipeline.ps1 -DryRun` | 0 | `dry-run` | written | validated |

### verify-release (full gate)

```
[1/6] secret scan                    OK
[2/6] cargo fmt --check              OK
[3/6] cargo clippy --workspace       OK
[4/6] cargo test --workspace         OK
[5/6] frontend npm.cmd test          OK
[6/6] frontend npm.cmd run build     OK
Release gate passed.
```

### Secret Scan

Repository secret scan passes with no allowlist:
```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\verify-release.ps1 -SecretScanOnly
# OK: secret scan found no matches in Git-tracked files.
```

### Workflow YAML Validation

```
ci-gates.yml:               Valid=True  Errors=0
release-host-evidence.yml:  Valid=True  Errors=0
```

### git diff --check

Clean — no whitespace errors.

## Artifact Evidence (Local Dry-Run)

| Runner | Manifest | Provenance | Hash Sidecars | SBOM Inventory | Archive Integrity |
|--------|----------|------------|---------------|----------------|-------------------|
| Windows host (`-DryRun`) | `manifest.json` (dry-run) | `provenance.json` | skipped (dry-run) | `dependency-inventory.json` | n/a |
| Android host (`-DryRun`) | `manifest.json` (dry-run) | `provenance.json` | skipped (dry-run) | `dependency-inventory.json` | n/a |

Both manifests pass `Assert-ReleaseManifestSchema`. Provenance notes explicitly
state: "Unsigned host provenance attestation for release evidence only." and
"This is not a SLSA, cosign, or in-toto signed attestation."

## Self-Audit Findings and Fixes

### Fix: Empty retention target crash (fail-closed)

**Found during audit:** Both runners called
`Remove-ReleaseRetentionTargets -Targets $targets` unconditionally in the
non-dry-run path. When `Get-ReleaseRetentionCleanupTargets` returned an empty
array (nothing to delete), PowerShell passed `$null` to the mandatory
`-Targets` parameter, crashing the run. This was a pre-existing latent bug in
the production runners that surfaced during real (non-dry-run) evidence
collection.

**Fix:** Added a null/empty guard before the call in both runners:
```powershell
if ($null -ne $targets -and @($targets).Count -gt 0) {
    Remove-ReleaseRetentionTargets -Root $artifactRoot -Targets $targets
}
```

Added a Pester regression test
(`ReleaseBuild retention empty-target guard`).

### Fix: Provenance Artifacts parameter null binding

**Found during testing:** When `$provArtifacts` was `@()` (empty collection in
dry-run mode), PowerShell bound `$null` to the `Mandatory=$true` `-Artifacts`
parameter of `New-ReleaseProvenance`, crashing the run.

**Fix:** Changed `New-ReleaseProvenance`'s `-Artifacts` parameter from
`Mandatory=$true` to `AllowEmptyCollection` (matching the existing pattern in
`New-ReleaseBuildManifest`), and added a null-coalesce guard at the call site.

### Remediation (post-review blockers)

Addressed review findings that would have blocked merge/CI:

1. **Upload package offline-verifiable:** `Copy-ReleaseEvidenceSubjects` stages
   binaries/APKs under `subjects/<kind>/` with matching `.sha256` sidecars inside
   the evidence directory. Provenance subjects now point at those staged paths.
   The Windows evidence job re-hashes subjects after staging before upload.
2. **YAML validation requires a real parser:** CI installs pinned
   `PyYAML==6.0.2`; local callers can instead use Node `yaml`/`js-yaml` when it
   is installed. Pure-PowerShell structural validation is test-only via explicit
   `-PreferPowerShell`, and the production path fails closed without a real parser.
3. **Ubuntu jobs:** install GTK/WebKit Tauri system deps and build `frontend/dist`
   before clippy/test. Secret-scan and workflow-syntax jobs run on Windows with
   `pwsh` (no bare `shell: powershell` on Ubuntu).
4. **Android job:** `rustup target add aarch64-linux-android` before host pipeline.
5. **Windows native stdout/stderr redacted** in `run-release-build.ps1` command
   runner (capture `2>&1`, re-emit through `Protect-ReleasePath`).
6. **Secret scan covers untracked build inputs** via `git ls-files --others`
   + `Find-ReleaseSecretPatternFindings` (path/rule only, never secret values).
   `verify-release.ps1` now reuses the production helper.
7. **`.sha256` written UTF-8 without BOM** via `UTF8Encoding($false)`.
8. **Pester 5:** `Run.PassThru = $true` set so result objects are returned.
9. **ZIP/APK integrity reads entry payloads**, not just entry names.
10. **The host-evidence workflow defaults to `-SkipBundle`** (`skip_bundle`
    default `true`); tags also host-only. This does not change the bare
    `run-release-build.ps1` default, which still requests a bundle unless passed
    `-SkipBundle`. Explicit workflow `skip_bundle=false` installs pinned
    `tauri-cli==2.11.2` via `cargo install --locked` before bundling.
11. **The public Node fallback selects and parses the requested workflow file**
    rather than its generated helper script when Python discovery is unavailable;
    Node-only valid and malformed-workflow tests cover that wrapper contract
    without invoking PyYAML. They do not claim third-party YAML-package
    compatibility beyond the `load`/`parse` interface.
12. **Untracked secret scan is fail-closed** on `git ls-files` failure, unread
    files, and inputs larger than 2 MiB (no silent skip).
    Host-global `core.excludesFile` is explicitly isolated so an unreadable
    personal ignore file cannot be mistaken for a repository input; repository
    ignore rules remain in force. A deterministic temporary-repository test
    proves a globally excluded secret is still caught while a committed
    repository ignore rule remains honored.

### Verified: No path/command output leakage

- Provenance JSON: no `C:\Users` paths (all redacted to `<HOME>` / `<REPO>` /
  `<EXTERNAL_PATH>`).
- Manifest JSON: no `C:\Users` paths, no secret patterns.
- Workflow YAML: no hardcoded workstation paths, no `storyforge-parallel-target`,
  no `C:\Users`.
- Command output: all runner output uses `Protect-ReleasePath` for display.
- Error output: both runners use `Get-ReleaseSafeErrorDetails` for catch blocks.

### Verified: Fail-closed posture

- `Assert-ReleaseManifestSchema` rejects: missing fields, invalid `build_status`,
  GUI/device acceptance != `not_claimed`, present artifacts without sha256.
- `Test-ReleaseArchiveIntegrity` throws on missing/corrupt archives.
- `Write-ReleaseHashFile` throws on missing artifacts.
- `Test-ReleaseWorkflowSyntax` throws on missing files, returns parse errors as
  results for malformed YAML.
- Android `-BuildApk` still fails before host builds when SDK/NDK missing
  (unchanged from prior workstream, verified by existing test).

### Verified: No stale artifact mis-collection

- Dry-run mode records artifacts as `skipped` status (not `present`), so they
  are not confused with real build output.
- Schema validation accepts `skipped` artifacts with null sha256 (correct).
- Non-dry-run mode uses `RequireFresh` flag on all artifact collection
  (unchanged from prior workstream).
- The existing freshness check (`Test-ReleaseArtifactIsFresh`) correctly
  rejected a pre-existing binary as stale for a new run during real testing.

## Unverified Items

1. **Remote Gitea Actions execution: UNVERIFIED.** No Gitea Actions runner is
   available in this environment. The workflows have not been triggered
   remotely. All validation is local (Pester, dry runs, verify-release, YAML
   syntax parse). Do not claim CI has run.

2. **Full Tauri Windows bundle (.msi/.nsis): UNVERIFIED.** Not built in this
   workstream. The `windows-host-evidence` job can produce it when triggered
   with `skip_bundle=false`, but no local bundle build was attempted.

3. **Android APK build: UNVERIFIED.** SDK/NDK are not installed. The
   `android-host-evidence` job runs host smoke only (frontend build,
   capabilities tests, aarch64 Rust check). No APK was produced or inspected.

4. **GUI/desktop acceptance: NOT IN SCOPE.** Explicitly `not_claimed` in all
   manifests and provenance records.

5. **Device acceptance: NOT IN SCOPE.** Explicitly `not_claimed`.

6. **Signed provenance/SBOM: NOT IN SCOPE.** The provenance is an unsigned
   host attestation. No cosign/sigstore/in-toto/SLSA signing was performed.

## Remaining Release Risks

| Risk | Severity | Mitigation |
|------|----------|------------|
| No Gitea runner verified | High | Deploy a runner and trigger the workflows; confirm artifact uploads and schema validation succeed remotely. |
| Tauri bundle not built | Medium | Trigger `release-host-evidence` with `skip_bundle=false` on a runner with Tauri CLI + WebView2. |
| Android APK blocked by SDK/NDK | High | Install Android SDK/NDK on the runner, then add `-BuildApk` to the Android job. |
| Workflow YAML schema not checked against Gitea Actions spec | Low | `Test-ReleaseWorkflowSyntax` validates YAML syntax only, not Gitea Actions job semantics. |
| `set-output` deprecation | Low | Gitea Actions may differ from GitHub Actions on `set-output` behavior; verify on runner. |

## Merge Advice

**Conditional yes** as a release CI evidence pipeline and provenance/schema
hardening slice. This workstream:

- Adds reproducible Gitea Actions gates and host evidence workflows.
- Extends production helpers (not YAML duplication) for provenance, hashes,
  archive integrity, and manifest schema validation.
- Adds 20 new Pester tests covering every new function.
- Fixes a latent empty-retention crash in both production runners.
- Preserves fail-closed behavior, strict `npm ci`, secret redaction, bounded
  retention, and current-run protection.

**Do NOT merge as a claim of:**
- Remote CI execution (unverified — no runner available).
- GUI or device acceptance (explicitly `not_claimed`).
- Signed/publishable artifacts (provenance is unsigned host attestation).
- Tauri bundle or APK evidence (not produced in this workstream).

**Recommended next steps before release publication:**
1. Deploy a Gitea Actions runner and trigger both workflows.
2. Verify artifact uploads contain manifest, provenance, hash sidecars, and
   SBOM inventory with non-dry-run `build_status=ok`.
3. Install Tauri CLI + WebView2 on the Windows runner for full bundle evidence.
4. Install Android SDK/NDK for APK evidence.

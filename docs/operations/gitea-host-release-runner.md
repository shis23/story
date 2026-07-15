# Gitea Host Release Runner (Credential-Free Ops)

> Scope: prepare a **host-side** Gitea Actions runner for StoryForge release
> evidence workflows. This document never embeds tokens, passwords, or
> registration secrets. It does **not** prove remote CI has run.

## What this proves / does not prove

| Claim | Status from this doc + local tools |
|-------|-------------------------------------|
| Local tool preflight for host evidence | Can prove with `scripts/test-release-runner-preflight.ps1` |
| Offline re-hash of an evidence package | Can prove with `scripts/verify-release-evidence.ps1` |
| Workflow YAML static governance | Can prove with Pester + `Assert-ReleaseWorkflowStaticContract` |
| Remote Gitea runner registered and jobs executed | **Not proven here** — requires separate operator action |
| Tauri GUI acceptance | **Never claimable** from host runners |
| Android device / true device APK acceptance | **Never claimable** without explicit device work outside this line |
| Signing / publish / upload of release products | **Out of scope** — do not do from this runner path |

## Workflows in this repository

| Workflow | Path | Purpose |
|----------|------|---------|
| Fast gates | `.gitea/workflows/ci-gates.yml` | fmt, clippy, test, frontend, Pester, secret scan, real YAML parser |
| Host evidence | `.gitea/workflows/release-host-evidence.yml` | Windows host + Android host smoke evidence only |

### Host-only defaults (fail closed)

- `release-host-evidence` defaults `skip_bundle=true` → production path uses
  `scripts/run-release-build.ps1 -SkipBundle`.
- Bundle evidence requires **explicit** `skip_bundle=false` **and** pinned
  `tauri-cli` install on the runner.
- The Windows producer receives `skip_bundle` as a controlled environment value,
  accepts only empty/`true`/`false`, and rejects any other manual/API value
  before starting a build. It is deliberately not interpolated into PowerShell
  source text.
- Android job does **not** pass `-BuildApk`. Requesting APK without
  `ANDROID_HOME` / `NDK_HOME` fails closed in the host pipeline.
- Artifacts use `retention-days: 14`.
- `permissions: contents: read` only. The repository static contract checks
  that declaration; confirm that the installed Gitea runner/version enforces
  it during remote preflight before treating it as an effective permission
  boundary.
- `concurrency` + `cancel-in-progress: true` per ref.
- Secret scan is fail closed; never dump env or tokens into logs.
- The Windows Pester job installs and imports pinned `PyYAML==6.0.2` before it
  runs parser-backed workflow-contract tests; do not rely on a globally
  preinstalled YAML package.

## Recommended runner labels

Use labels that match the workflow `runs-on` values your Gitea instance maps
to self-hosted capacity. Typical mapping:

| Workflow job | Workflow `runs-on` | Suggested self-hosted labels |
|--------------|--------------------|------------------------------|
| `ci-gates` Linux jobs | `ubuntu-latest` | `ubuntu-latest`, `linux`, `x64` |
| `ci-gates` Windows jobs | `windows-latest` | `windows-latest`, `windows`, `x64` |
| `release-host-evidence` | `windows-latest` | `windows-latest`, `windows`, `x64`, `release-host` |

Do **not** put credentials in label names.

## Least privilege (host)

Minimum host permissions for a dedicated release evidence runner:

1. Run as a non-admin service account when possible.
2. Read access to the cloned workspace only; no shared secrets volume.
3. Write access limited to:
   - runner work directory
   - cargo/npm caches under the service account home
   - `artifacts/release-build/` inside the workspace
4. No cloud publish credentials, no Android keystore, no code-signing certs.
5. Network: only what is required to fetch dependencies and talk to Gitea;
   never embed tokens in repo files or workflow logs.
6. Do **not** mount production data directories into the runner.

## Cache guidance

- Prefer action-native npm cache (`actions/setup-node` + `frontend/package-lock.json`).
- Cargo registry/git caches may live under the service account home; never
  point `CARGO_TARGET_DIR` at a developer workstation path.
- Workflows set `CARGO_TARGET_DIR: ''` so a laptop path cannot leak in.
- Clear stale caches on runner rotation; caches are convenience, not trust.

## Artifact retention and concurrency

- Uploaded evidence packages: **14 days** (`retention-days: 14`).
- Local `artifacts/release-build/` retention is controlled by the PowerShell
  runners (`-KeepRuns`, default 5).
- Concurrent runs on the same ref cancel predecessors (`cancel-in-progress`).

## Manual trigger (no credentials in repo)

1. Ensure a runner with the labels above is online in **your** Gitea UI
   (registration token is obtained from Gitea UI and never committed).
2. Open the repository → Actions → `release-host-evidence`.
3. Run `workflow_dispatch`:
   - leave `skip_bundle=true` for host-only evidence (default);
   - set `skip_bundle=false` only after `cargo-tauri` is installed and you
     intentionally want bundle evidence.
   - an empty tag-trigger input remains host-only; any other malformed value
     must fail the producer before it executes a build.
4. After the job finishes, download the artifact zip and verify offline:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\verify-release-evidence.ps1 `
  -EvidenceDir .\path\to\extracted\evidence
```

Dry-run local packages (not remote CI) require `-AllowDryRun`.

## Local preflight (before registering a runner)

```powershell
# Host-only Windows evidence tools
powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\test-release-runner-preflight.ps1 `
  -Profile windows-host

# Explicit bundle intent (fails closed without cargo-tauri)
powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\test-release-runner-preflight.ps1 `
  -Profile windows-host -RequireBundle

# Android host smoke (no APK)
powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\test-release-runner-preflight.ps1 `
  -Profile android-host

# Explicit APK intent (fails closed without ANDROID_HOME/NDK_HOME)
powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\test-release-runner-preflight.ps1 `
  -Profile android-host -RequireApk

# CI gate profile (includes python + PyYAML for workflow syntax)
powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\test-release-runner-preflight.ps1 `
  -Profile ci-gates
```

Reports declare:

- `claims.gui = not_claimable`
- `claims.android_device = not_claimable`
- `claims.remote_ci = not_claimable`

Missing dependencies, unauthorized bundle/APK, or missing YAML parser **fail closed**.

## Tooling checklist (Windows host evidence)

| Tool | Host-only | Bundle | APK | CI syntax job |
|------|-----------|--------|-----|---------------|
| PowerShell 5+ / pwsh | required | required | required | required |
| rustc + cargo | required | required | required | required |
| Node 20 + npm (`npm ci`) | required | required | required | required |
| cargo-tauri (pinned in workflow when requested) | not required | **required** | optional | no |
| ANDROID_HOME + NDK_HOME | no | no | **required** | no |
| aarch64-linux-android target | android host | no | required | no |
| Python 3 + PyYAML 6.0.2 | optional locally | optional | optional | **required** |
| GTK/WebKit (Linux clippy/test jobs only) | n/a on Windows host | n/a | n/a | Linux jobs |

## Fail-closed rules operators must not weaken

1. Missing tool for the selected profile → non-zero exit / failed job.
2. Hash mismatch, missing subject, or missing `.sha256` sidecar → reject package.
3. UTF-8 BOM on sidecars → reject.
4. Path escape (`..`, absolute paths outside package) → reject.
5. Unknown schema version → reject.
6. Secret-like tokens in manifest/provenance notes/warnings → reject (values redacted).
7. Structural-only YAML checks are **not** accepted for production workflow validation.
8. Never treat local dry-run as remote CI success.
9. Do not add `uses` and `run` to the same governed step, or job/workflow
   shell/container/service/environment overrides: the static contract rejects
   those ambiguous execution contexts. The sole approved step environment is
   the Windows producer's exact `SF_RELEASE_SKIP_BUNDLE_INPUT` binding.
10. The workflow action allowlist uses version tags/labels, not immutable
    commit-SHA pins. Protect refs and verify the installed runner/action supply
    chain separately.

## Registration note (token handling)

Gitea runner registration tokens are **short-lived operator secrets**:

- Obtain from Gitea UI or admin API using your own access.
- Pass to the runner install process via environment or local config **outside git**.
- Never commit `.runner`, tokens, or `config.yaml` containing secrets.
- Rotate tokens if leaked; this repository must remain free of credentials.

## Related scripts

| Script | Role |
|--------|------|
| `scripts/test-release-runner-preflight.ps1` | Local readiness report |
| `scripts/verify-release-evidence.ps1` | Offline evidence verifier |
| `scripts/run-release-build.ps1` | Windows host evidence runner |
| `scripts/run-android-host-pipeline.ps1` | Android host smoke (+ optional `-BuildApk`) |
| `scripts/tests/run-release-build-tests.ps1` | Pester suite including readiness tests |

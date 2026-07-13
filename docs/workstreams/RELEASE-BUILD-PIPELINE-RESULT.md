# Release Build and Android Host Pipeline Result

- Branch: `codex/release-build-pipeline`
- Baseline: `main@c3a972d`
- Worktree: `C:\tmp\storyforge-release-build`
- Date: 2026-07-13
- Closing HEAD: see commit list below (`git rev-parse --short HEAD`)

## Scope completed

1. Windows release build runner with clean-input note, frontend/Rust release steps, expected binary checks, fail-closed exit codes.
2. Deterministic host artifact manifests (commit/branch/target/tool versions/size/SHA-256/status) with path/secret redaction; GUI and Android device acceptance explicitly `not_claimed`.
3. Dependency/license-style inventory via `cargo metadata` (+ package-lock when present), with documented lockfile fallback.
4. Size budgets/warnings for Windows binaries/bundles and Android APKs; over-budget is warning only, never silent acceptance.
5. Android host-side smoke re-run (frontend build, capabilities test, `aarch64-linux-android` infra-util check). APK builds require `ANDROID_HOME` + `NDK_HOME`; missing NDK fails closed.
6. Gradle/Kotlin/Tauri/proguard warning normalization helper for APK build logs.
7. Retention cleanup for `artifacts/release-build` (default keep 5 runs).
8. PowerShell parser/unit/pipeline tests proving dry-run, redaction, hashing, budgets, missing tools/artifacts fail closed.

## Commits (this line)

1. `6e8e1b2` docs(workstream): plan release build pipeline
2. `f0e7531` test(release-build): add fail-closed helpers and Pester coverage
3. `58316ce` feat(release-build): Windows and Android host evidence runners
4. `63b1205` test(release-build): avoid static secret-scan false positives in fixtures
5. `0a63cf7` fix(release-build): harden host runners for local evidence collection
6. (this RESULT commit, if present on tip)

## Modified / added files

- `scripts/release-build/ReleaseBuild.Common.ps1` — shared helpers
- `scripts/run-release-build.ps1` — Windows host release runner
- `scripts/run-android-host-pipeline.ps1` — Android host evidence pipeline
- `scripts/tests/ReleaseBuild.Tests.ps1` — unit tests
- `scripts/tests/ReleaseBuild.Pipeline.Tests.ps1` — parser/dry-run/fail-closed tests
- `scripts/tests/run-release-build-tests.ps1` — Pester entry
- `docs/workstreams/RELEASE-BUILD-PIPELINE-PLAN.md` — plan (pre-existing on branch)
- `docs/workstreams/RELEASE-BUILD-PIPELINE-RESULT.md` — this result

Not tracked (generated outside Git, already gitignored via `artifacts/`):

- `artifacts/release-build/windows-*/manifest.json`
- `artifacts/release-build/windows-*/dependency-inventory.json`
- `artifacts/release-build/windows-*/SUMMARY.txt`
- `artifacts/release-build/android-*/manifest.json`
- `artifacts/release-build/android-*/apk-inspection.json`
- `artifacts/release-build/android-*/SUMMARY.txt`
- local `frontend/dist` placeholder (when `-SkipFrontend`)
- `target/release/storyforge.exe` / `storyforge_lib.dll`

## Red → green evidence

| Step | Result |
| --- | --- |
| Initial Pester without helpers | RED: missing `ReleaseBuild.Common.ps1` |
| Helpers + unit tests | GREEN: 20/20 |
| Pipeline dry-run before scope/type fixes | RED: function-scope import / List cast issues |
| After script-scope import + array casts | GREEN: parser + dry-run + fail-closed (29 total assertions across both files) |
| Secret-scan fixture false positive | RED on `verify-release -SecretScanOnly` for local `sk-...` fixture |
| Runtime-constructed fake token | GREEN for release-build workstream files (0 hits) |

Commands:

```text
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/tests/run-release-build-tests.ps1
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/run-release-build.ps1 -DryRun
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/run-android-host-pipeline.ps1 -DryRun
```

## Actual host results (this machine)

### PowerShell gates

| Gate | Command | Result |
| --- | --- | --- |
| Unit + pipeline tests | `scripts/tests/run-release-build-tests.ps1` | **PASS** (20 unit + 9 pipeline) |
| Windows dry-run | `scripts/run-release-build.ps1 -DryRun` | **PASS** status=`dry-run` |
| Android dry-run | `scripts/run-android-host-pipeline.ps1 -DryRun` | **PASS** status=`dry-run` |
| Whitespace | `git diff --check c3a972d..HEAD` | **PASS** |
| Secret scan (workstream files only) | pattern scan of new scripts/tests | **PASS** (0 hits) |
| Full `verify-release -SecretScanOnly` | repo-wide | **FAIL pre-existing**: `crates/harness-real-llm/tests/eval_m5_phaseb_deterministic.rs:176` fixture `sk-test-should-not-write` (outside this line; not introduced here) |

### Windows host build

Command:

```text
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/run-release-build.ps1 -SkipFrontend -SkipBundle
```

Result: **host status=ok** (not GUI acceptance).

Evidence dir: `artifacts/release-build/windows-20260713-114819`

| Artifact | Status | Size (bytes) | SHA-256 |
| --- | --- | --- | --- |
| `target/release/storyforge.exe` | present | 25495552 | `d26ff13c133fd8bccbd0ba8bc5b93a180fde982205d85138655db6bf4a5ef83f` |
| `target/release/storyforge_lib.dll` | present | 435200 | `ed258a3895e1b820a7127c6d231301f6a658b3df11b184f1581232bafd8a8d10` |

Notes from that run:

- working tree was dirty during evidence collection (script notes dirty inputs; does not claim clean-input release)
- local `frontend/dist` placeholder created because `-SkipFrontend` and dist was missing
- dependency inventory generator: `cargo-metadata` (later dry-run samples show ~573 components including transitive crates + package-lock)
- acceptance: `gui=not_claimed`, `android_device=not_claimed`

Size budgets (soft warnings only):

- `windows-exe` budget 80MB — observed ~24.3MB → ok
- `windows-msi` / `windows-nsis` not produced (bundle skipped / tauri bundle not claimed)

### Android host smoke

Command:

```text
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/run-android-host-pipeline.ps1
```

Result: **host status=ok** (not device acceptance).

Environment observed:

- `ANDROID_HOME` set and exists
- `NDK_HOME` **not set**
- `JAVA_HOME` set
- adb/device not required and not used

Steps completed:

1. `frontend npm.cmd ci` / `npm.cmd run build`
2. `cargo test -p storyforge --test capabilities` (2 passed)
3. `cargo check -p storyforge-infra-util --target aarch64-linux-android`

### Android APK path (fail-closed)

Command:

```text
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/run-android-host-pipeline.ps1 -BuildApk
```

Result: **FAIL closed** as designed:

```text
Android APK build environment is incomplete:
  NDK_HOME is required for -BuildApk but is not set.
```

No APK signing, publishing, device install, or GUI launch was attempted.

## Cleanup / retention

- Evidence roots under `artifacts/release-build/` (gitignored)
- Runners support `-KeepRuns` (default 5) and delete older run directories
- Observed retention removing older windows/android dry-run dirs during later runs
- No keystores, APKs, or large build trees committed

## Unfinished / risks

| Item | Status | Risk |
| --- | --- | --- |
| Full Windows `cargo tauri build` bundle (.msi/.nsis) | Not completed in this pass (`-SkipBundle` used for scoped host binary evidence) | Medium for installer evidence |
| Frontend production build as part of non-skip Windows runner | Implemented; this machine evidence used `-SkipFrontend` + placeholder for compile context | Medium — re-run without skip on a clean machine |
| Android debug/release APK build + ABI/SQLite inspection on real APKs | Blocked by missing `NDK_HOME` | High for Android package evidence; host smoke still ok |
| Normalized Gradle/Kotlin/proguard warning report from real APK logs | Helper + tests present; no real APK log captured this host | Low until APK build runs |
| Repo-wide secret scan | Pre-existing eval fixture false-positive remains | Process — fix on eval line or allowlist policy |
| GUI / physical device acceptance | Explicitly out of scope | Do not treat host ok as release PASS |

## Merge recommendation

**Conditional yes: merge as a host-side release evidence pipeline slice.**

Reasons:

- Fail-closed PowerShell tests and dry-runs are green
- Windows release binary evidence collected with hashes/sizes and non-claiming acceptance fields
- Android host smoke re-validated; APK path correctly fails closed without NDK
- Generated binaries/APKs stay outside Git

Do **not** merge-as-claim that:

- desktop GUI is accepted
- Android device install/runtime is accepted
- signed/publishable installers or APKs exist from this line

## Commands for integrators

```text
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/tests/run-release-build-tests.ps1
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/run-release-build.ps1 -DryRun
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/run-android-host-pipeline.ps1 -DryRun
# full host (when node_modules/SDK ready):
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/run-release-build.ps1
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/run-android-host-pipeline.ps1
# APK only when ANDROID_HOME and NDK_HOME exist:
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/run-android-host-pipeline.ps1 -BuildApk
git diff --check c3a972d..HEAD
```

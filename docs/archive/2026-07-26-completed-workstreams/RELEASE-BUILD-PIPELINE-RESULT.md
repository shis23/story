# Release Build and Android Host Pipeline Result

- Branch: `codex/release-build-pipeline`
- Baseline: `main@c3a972d`
- Worktree: `C:\tmp\storyforge-release-build`
- Date: 2026-07-13
- Closing HEAD: see commit list below (`git rev-parse --short HEAD`)

## Scope completed

1. Windows release build runner with clean-input note, frontend/Rust release steps, expected binary checks, fail-closed exit codes.
2. Deterministic host artifact manifests (commit/branch/target/tool versions/size/SHA-256/status) with recursive path/secret redaction; GUI and Android device acceptance explicitly `not_claimed`.
3. Dependency/license-style inventory via `cargo metadata` (+ package-lock when present), with documented lockfile fallback and manifest binding by relative path, SHA-256, component count, and generator.
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
6. `20a2c5c` docs(workstream): record release build pipeline RESULT
7. `f0dc74e` docs(workstream): avoid secret-scan false positive in RESULT text
8. `7addb06` docs(workstream): finalize release build RESULT commit list
9. `22e1c85` fix(release-build): close fail-closed gaps in host evidence runners
10. `86133e4` fix(release-build): close final fail-closed evidence gaps
11. `459fd86` fix(release-build): sanitize command and failure output

## Modified / added files

- `scripts/release-build/ReleaseBuild.Common.ps1` — shared helpers
- `scripts/run-release-build.ps1` — Windows host release runner
- `scripts/run-android-host-pipeline.ps1` — Android host evidence pipeline
- `scripts/tests/ReleaseBuild.Tests.ps1` — unit tests
- `scripts/tests/ReleaseBuild.Pipeline.Tests.ps1` — parser/dry-run/fail-closed tests
- `scripts/tests/run-release-build-tests.ps1` — Pester entry
- `crates/harness-real-llm/tests/eval_m5_phaseb_deterministic.rs` — runtime construction for the hostile secret fixture, keeping the strict scan allowlist-free
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
| Helpers + unit tests | GREEN: 37/37 |
| Pipeline dry-run before scope/type fixes | RED: function-scope import / List cast issues |
| After script-scope import + array casts | GREEN: parser + dry-run + fail-closed |
| Final review tests | RED: 9 failures covering manifest recursion, APK contract, retention junction/delete safety, path boundary, run-id collision, and Pester result policy |
| Final review implementation | GREEN: 37 unit + 14 pipeline; production missing-NDK child process exits before npm/cargo |
| Secret-scan fixture false positive | RED on the strict default repository scan for a static synthetic key-shaped fixture |
| Runtime-constructed hostile fixture | GREEN for both worktree and staged index; no allowlist or `-SkipSecretScan` required |

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
| Unit + pipeline tests | `scripts/tests/run-release-build-tests.ps1` | **PASS** (37 unit + 14 pipeline; 0 skipped/pending/inconclusive) |
| Windows dry-run | `scripts/run-release-build.ps1 -DryRun` | **PASS** status=`dry-run` |
| Android dry-run | `scripts/run-android-host-pipeline.ps1 -DryRun` | **PASS** status=`dry-run` |
| Whitespace | `git diff --check c3a972d..HEAD` | **PASS** |
| Default runner secret scan | worktree + staged index, repo-wide | **PASS** (0 hits; no allowlist) |
| Deterministic harness fixture regression | `cargo test -p harness-real-llm --test eval_m5_phaseb_deterministic` | **PASS** (5/5) |

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

## Fail-closed hardening follow-up (`22e1c85`)

Review findings closed in-branch:

| Finding | Fix |
| --- | --- |
| Windows bundle / Android APK failure degraded to `partial` with exit 0 | Requested bundle/APK paths now fail closed; `partial`/`failed` exit non-zero via `Get-ReleaseProcessExitCode` |
| Stale installer/APK harvest attributed to current SHA | Artifact collection requires freshness vs `build_started_utc`; Android only scans current Gradle outputs |
| Retention deleted arbitrary dirs under evidence root | Only `windows-*` / `android-*` prefixes; skip reparse points; protect current run dir |
| Secret scan not in runners | `Invoke-ReleaseSecretScan` runs by default; `-SkipSecretScan` is explicit |
| Android fail-closed tests used hard-coded strings | Tests call `Get-ReleaseAndroidBuildPathIssues` / `Assert-ReleaseAndroidBuildEnvironment` |
| `npm ci` fell back to `npm install` | Fallback removed; reproducible `npm ci` only |
| Manifest warnings/notes not redacted | `New-ReleaseBuildManifest` redacts warnings/notes with `Protect-ReleasePath` |

## Final review hardening (`86133e4`)

| Finding | Fix / executable evidence |
| --- | --- |
| Retention root could itself be a junction and deletes suppressed errors | Reject reparse roots/targets, require a direct child inside the trusted root, propagate deletion errors, and recheck disappearance; junction and outside-root tests pass |
| Manifest and Android evidence sanitized only selected fields | Recursively sanitize every string value; Android captured logs are sanitized before disk/console; external paths become `<EXTERNAL_PATH>` |
| Dependency inventory was adjacent evidence but not bound to the manifest | Manifest records inventory relative path, SHA-256, component count, and generator |
| APK evidence accepted one variant or an uninspectable/non-arm64 archive | Requested APK builds require fresh debug **and** release kinds; ZIP inspection and arm64 native evidence fail closed |
| SQLite APK detection matched arbitrary substrings | Match exact native `libsqlite3.so` / `libsqlcipher.so` paths only |
| `npm ci` was skipped when `node_modules` already existed | Both production runners always execute `npm ci` before frontend build |
| Run directories collided within one second | Millisecond timestamp plus GUID nonce; creation is non-overwriting |
| Pester could report success with zero executed or skipped tests | Entrypoint requires total > 0 and zero failed/skipped/pending/inconclusive tests |
| Missing Android prerequisites were checked after host build work | Production `-BuildApk` preflight now exits non-zero before npm/cargo; child-process test covers the runner path |

### Failure-output redaction follow-up (`459fd86`)

- `Protect-ReleasePath` now removes `Authorization: Bearer ...` and standalone
  Bearer credentials in addition to the existing token/key patterns.
- Both runners sanitize displayed command text and route exception message,
  script stack, and invocation position through one tested
  `Get-ReleaseSafeErrorDetails` helper.
- Top-level failure output uses direct stderr text rather than `Write-Error`,
  avoiding PowerShell adding an unsanitized script path to a new error record.
- Synthetic credential + Windows user-path RED/GREEN tests contain no real key;
  the strict default repository scan remains green.

## Unfinished / risks

| Item | Status | Risk |
| --- | --- | --- |
| Full Windows `cargo tauri build` bundle (.msi/.nsis) | Not completed in this pass (`-SkipBundle` used for scoped host binary evidence) | Medium for installer evidence |
| Frontend production build as part of non-skip Windows runner | Implemented; this machine evidence used `-SkipFrontend` + placeholder for compile context | Medium — re-run without skip on a clean machine |
| Android debug/release APK build + ABI/SQLite inspection on real APKs | Blocked by missing `NDK_HOME` | High for Android package evidence; host smoke still ok |
| Normalized Gradle/Kotlin/proguard warning report from real APK logs | Helper + tests present; no real APK log captured this host | Low until APK build runs |
| Repo-wide secret scan | **Closed**: strict default scan passes after runtime fixture construction | Keep `-SkipSecretScan` exceptional; no allowlist added |
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
# full host (when package-registry access and toolchains are ready):
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/run-release-build.ps1
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/run-android-host-pipeline.ps1
# APK only when ANDROID_HOME and NDK_HOME exist:
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/run-android-host-pipeline.ps1 -BuildApk
git diff --check c3a972d..HEAD
```

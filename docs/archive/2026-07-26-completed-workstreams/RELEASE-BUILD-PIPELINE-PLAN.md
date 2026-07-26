# Release Build and Android Host Pipeline Plan

- Branch: `codex/release-build-pipeline`
- Baseline: `main@c3a972d`
- Worktree: `C:\tmp\storyforge-release-build`
- No GUI interaction, signing, publishing or physical device

## Objective

Create a reproducible, fail-closed release build pipeline for Windows and Android host-side
artifacts. Produce manifests, size/security evidence and actionable warnings without claiming
desktop GUI or Android device acceptance.

## Required scope

1. Add a Windows release build runner that checks clean inputs, frontend build, Rust release
   build/Tauri bundle availability, expected binaries and exit codes.
2. Generate a deterministic artifact manifest containing commit, tool versions, target, size,
   SHA-256 and build status; never include environment secrets or absolute user paths.
3. Generate dependency/license or SBOM-style inventory using available local tooling, with a
   documented fallback if a tool is unavailable.
4. Add size budgets/warnings for Windows binaries/bundles and Android APKs; warnings must not be
   silently treated as acceptance.
5. Re-run Android arm64 host-side checks and, when the installed SDK permits, debug/release APK
   builds. Inspect APK ABI entries, native libraries, bundled SQLite presence and capabilities.
6. Record Gradle/Kotlin/Tauri deprecation and proguard warnings in a normalized report.
7. Add cleanup/retention controls so artifacts and Gradle/Cargo intermediates do not fill C:.
8. Add script tests/dry-runs proving missing tools, failed builds and missing artifacts fail closed.

## TDD and acceptance

- PowerShell parser and dry-run tests before invoking heavy builds.
- Manifest hashing and redaction tests.
- A successful host build is not a GUI/device PASS; RESULT must keep those states separate.
- Repository must not track generated binaries, APKs, keystores or large build directories.

## Boundaries

- Do not start the desktop GUI.
- Do not install to or access a physical Android device.
- Do not sign, publish or upload artifacts.
- Do not call a real LLM or alter storage/model defaults.
- Avoid source-feature work outside fixes strictly required for reproducible building.
- Do not edit `docs/HANDOFF.md`.

## Scoped gates

- PowerShell script parser/unit tests and dry-runs
- frontend test/build where required
- relevant Rust release/check targets
- Android host smoke/build only when local SDK prerequisites exist
- secret scan, manifest validation and `git diff --check c3a972d..HEAD`

## Delivery

Commit coherent steps, do not push, and create
`docs/workstreams/RELEASE-BUILD-PIPELINE-RESULT.md` with commands, artifacts generated outside
Git, hashes/sizes, warnings, host-only status, missing prerequisites and cleanup results.

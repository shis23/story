# Release CI and Reproducible Evidence Plan

> Branch: `codex/release-ci-evidence`
> Worktree: `C:\tmp\storyforge-release-ci`
> Base: `b46ddc8`

## Objective

Convert the host-side release runners into a repeatable Gitea Actions evidence
pipeline with bounded jobs, deterministic manifests, artifact retention, and
truthful Windows/Android host claims. This line does not perform publishing,
signing, GUI acceptance, or physical-device acceptance.

## Scope

### Gitea Actions

- Add `.gitea/workflows` jobs for formatting, strict Clippy, workspace tests,
  frontend tests/build, Pester release tests, secret scan, and host artifacts.
- Use explicit timeouts, concurrency cancellation, least-privilege permissions,
  and pinned action versions/commit SHAs where supported.
- Separate fast PR gates from manual/tagged artifact jobs.
- Fail closed when required Windows/Android host prerequisites or expected
  artifacts are missing.
- Never expose secret values in commands, environment dumps, logs, annotations,
  cache keys, artifact names, or manifests.

### Build and Provenance

- Produce deterministic Windows host artifact manifests with commit, branch,
  tool versions, target, size, SHA-256, status, warnings, and acceptance scope.
- Generate SBOM/dependency inventory using available local tooling; fail clearly
  or mark unsupported without fabricating evidence.
- Verify artifact freshness from the current build start and current commit.
- Add archive integrity verification and manifest schema tests.
- Keep generated artifacts ignored and use bounded retention.

### Android Host Evidence

- Run frontend build, capabilities, aarch64 Rust check, Gradle/project checks,
  and optional APK build only when SDK/NDK prerequisites are present.
- Inspect APK ABI/package/size/hash when produced.
- Distinguish host build, unsigned APK, signed APK, install, device acceptance,
  and release publication. Only the first two are in scope.
- Do not install SDK/NDK automatically or use a physical device.

### Runner Hardening

- Preserve strict `npm ci` behavior.
- Re-sanitize command, native tool, warning, note, and failure output.
- Include tracked/untracked build-input policy and repository cleanliness in the
  manifest.
- Keep retention limited to release evidence roots and protect the active run.
- Add dry-run/parser tests that execute production helper functions.

## Validation

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\tests\run-release-build-tests.ps1
powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\run-release-build.ps1 -DryRun
powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\run-android-host-pipeline.ps1 -DryRun
powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\verify-release.ps1
git diff --check
```

Validate workflow YAML/schema with available tooling. If no Actions runner is
available, mark remote execution unverified; do not claim CI has run.

## Deliverables

- Gitea Actions workflow(s);
- hardened reusable release scripts and tests;
- provenance/manifest/SBOM schema and verification;
- Windows and Android host evidence jobs;
- retention and secret/path redaction tests;
- `docs/workstreams/RELEASE-CI-EVIDENCE-RESULT.md`;
- logical commits and a clean worktree.

## Prohibited Changes

- no signing keys, publishing, releases, or deployment;
- no GUI or physical-device acceptance claims;
- no real LLM work;
- no edits to `docs/HANDOFF.md` or `docs/RELEASE-CHECKLIST.md`;
- do not edit SQLite, plugin, import-corpus, or writing pipeline code;
- no push, rebase, force push, or modification of `main`.

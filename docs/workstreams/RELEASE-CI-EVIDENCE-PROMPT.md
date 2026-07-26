# ZCode Prompt: Release CI and Reproducible Evidence

Work only in:

```text
C:\tmp\storyforge-release-ci
branch codex/release-ci-evidence
```

For local Rust validation, set `$env:CARGO_TARGET_DIR` to
`C:\tmp\storyforge-parallel-target`. Do not run `cargo clean`; wait for shared
Cargo locks. CI configuration must not hard-code this workstation path.

Read:

1. `docs/workstreams/RELEASE-CI-EVIDENCE-PLAN.md`
2. `docs/archive/2026-07-26-completed-workstreams/RELEASE-BUILD-PIPELINE-RESULT.md`（2026-07-26 归档）
3. `docs/workstreams/RELEASE-BRONZE-RESULT.md`
4. current release scripts and Pester tests.

Implement the full PLAN with tests first. Add Gitea Actions workflows for fast
gates and manual/tagged host artifact evidence. Use timeouts, concurrency
cancellation, minimal permissions, pinned actions where supported, and strict
secret handling.

Extend the existing production helpers rather than duplicating them in YAML.
Generate and verify Windows/Android host manifests, hashes, provenance, archive
integrity, and SBOM/dependency inventory. Artifacts must be fresh for the current
build/commit. Missing requested bundles/APKs or prerequisites must fail closed.

Keep host build, unsigned APK, signed APK, install/device, GUI acceptance, and
publication as separate states. This branch may only claim host evidence and an
unsigned APK when actually produced. Do not install SDK/NDK or use a device.

Re-sanitize commands, native outputs, warnings, notes, errors, paths, and
manifests. Preserve `npm ci`, strict repository secret scan, bounded retention,
and current-run protection. Add production-helper Pester coverage, dry runs, and
workflow syntax/schema checks.

Do not edit `docs/HANDOFF.md` or `docs/RELEASE-CHECKLIST.md`. Do not touch real
LLM, SQLite, plugin, import-corpus, or writing pipeline code. Do not sign,
publish, deploy, or push.

Make logical commits, keep the worktree clean, run all available PLAN gates,
and finish with `docs/workstreams/RELEASE-CI-EVIDENCE-RESULT.md`. If no Gitea
Actions runner is available, state that remote execution is unverified rather
than claiming PASS.

Report branch, HEAD, commits, workflow jobs, local gates, artifact evidence,
secret scan, unsupported environment items, remaining release risks, and merge
advice.

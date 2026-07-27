# Release CI Live Execution Result (2026-07-27)

> Branch: `codex/release-ci-live`
> Worktree: `C:\Users\Predator\ZCodeProject\storyforge\.worktrees\release-ci-live`
> Base SHA: `29513a600a404563ef5aad30ad52fa97b3d0c90a` (latest local `main`)
> Head SHA (after rework): see `git rev-parse HEAD` on this branch
> Date: 2026-07-27 (initial audit), reworked same day
> Owner line: Release CI (`.gitea/workflows/**`, `scripts/run-release-*`, `scripts/verify-release*`, Release CI/runner RESULT docs)

## Objective

Advance the Release CI line to **real, current Gitea Actions remote execution**
that reaches a terminal state, with reviewable evidence and honest status. This
iteration also fixed two defects found during the first audit: (1) the Windows
`if:` guard left ci-gates runs permanently non-terminal, and (2) the secret
scanner produced false positives on story-task identifiers and test fixtures.

No GUI, no real LLM, no signing, no release publication. Default no push.

## Headline

**PARTIAL — rework landed; remote new run NOT yet produced (no push
authorization).**

- The non-terminal-run defect is fixed: Windows gates split into
  `windows-gates.yml`; ci-gates now only runs Linux jobs that always finish.
- The secret scanner no longer false-positives; `verify-release -SecretScanOnly`
  is green locally.
- **No new remote run exists.** The only remote evidence remains the historical
  run #19 on `origin/main@d9bc4e5`, which predates this branch and does NOT
  exercise this branch's code. Status stays PARTIAL/BLOCKED until a fresh run
  is triggered on this branch's head.

## Repository and SHAs

| Item | Value |
| --- | --- |
| Repo (origin) | `https://git.2529985.xyz/ss/story.git` |
| Base (this branch) | `29513a600a404563ef5aad30ad52fa97b3d0c90a` |
| `origin/main` (pushed HEAD) | `d9bc4e54b78f3253c89e8a366a76259fa3185870` |
| Code under test for a NEW run | this branch's head (see `git rev-parse HEAD`) |
| Historical remote run #19 commit | `d9bc4e5` (`origin/main`) — **not** this branch |

The 5 unpushed local commits on `main` (d452ce4..29513a6) and this branch's
rework commits are owned by other workstream lines or this line; none are
pushed, so remote CI has not exercised them.

## Commits on this branch

| # | Hash | Subject |
| --- | --- | --- |
| 1 | `3bd6eae` | docs(release-ci): record live Gitea Actions remote execution evidence (initial audit) |
| 2 | `be0f55b` | feat(release-scanner): boundary-aware secret matching |
| 3 | `b96c1b2` | feat(ci): split Windows gates into windows-gates.yml for terminal runs |
| 4 | (this commit) | docs(release-ci): update LIVE-RESULT after rework |

## Runner state (non-sensitive facts only)

Queried via the runner host (`/opt/act_runner/ci-status.sh`) and the Gitea
`action_runner` table. No tokens, hashes, salts, or secrets are recorded here.

| Field | Value |
| --- | --- |
| Runner count | 1 |
| Runner id / name | 1 / `jd-linux-1` |
| `act_runner` version | v0.6.1 |
| Labels | `["ubuntu-latest","ubuntu-22.04"]` |
| `is_disabled` | 0 (enabled) |
| Windows runner | **None registered** |
| Repo Actions variable `HAS_WINDOWS_RUNNER` | unset |

Gitea `[actions]` config: `ENABLED = true`, `DEFAULT_ACTIONS_URL = self`.
Gitea version 1.26.1.

## Defect 1 (fixed): Windows `if:` guard kept runs non-terminal

**Root cause** (proven by run #19): on Gitea 1.26 + act_runner v0.6.x, a
job-level `if: vars.HAS_WINDOWS_RUNNER == 'true'` combined with
`runs-on: windows-latest` is **not** evaluated before runner assignment. With
no Windows runner, the job stays at `status=5` (blocked/waiting) and never
becomes `skipped`, so the whole ci-gates run never finishes and the API refuses
`rerun` ("this workflow run is not done").

Run #19 on `d9bc4e5`: the four Linux jobs (`rust-fmt`, `rust-clippy`,
`rust-test`, `frontend-gate`) all reached `status=1` (success) on `jd-linux-1`;
the three Windows jobs (`pester-release-tests`, `secret-scan`, `workflow-syntax`)
sat at `status=5` forever; run-level status stayed `5`, `stopped=0`.

**Fix (commit b96c1b2):** the three Windows jobs moved to a new
`windows-gates.yml` triggered only by `workflow_dispatch` and `push: tags: v*`,
with **no** job-level `if:` guard. ci-gates.yml now contains only the four Linux
jobs plus a `workflow_dispatch` trigger for controlled re-runs without a
business commit. With no Windows runner: dispatching `windows-gates` queues for
a runner (and does not silently pass); ordinary push/PR ci-gates always finish.
`release-host-evidence.yml` is unchanged.

ci-gates.yml header comment now states the truth: Linux gates only; Windows
gates live in windows-gates.yml. The earlier "Windows jobs gated off as
designed" wording is removed — it contradicted the permanent `status=5` reality.

## Defect 2 (fixed): secret scanner false positives (commit be0f55b)

`verify-release -SecretScanOnly` failed closed on two classes of false
positives, both pre-existing in other lines' files:

1. **`sk-` embedded in identifiers.** Story-task ids `task-authenticate-red-wax-note`
   and `task-follow-gold-raven-decoy` matched the OpenAI-key rule
   `sk-[A-Za-z0-9_-]{20,}` because the scanner matched the `sk-` substring
   inside the identifier.
2. **`secret:` / `api_key:` / `token:` assignments in test fixtures and docs.**
   Rust struct literals (`secret: "SF_SECRET_CHEN_BADGE_X91".into()`), sentinel
   values (`SF_SECRET_should_be_stripped`, `EARLYFACT-ZXQ-7719`), English-phrase
   values, angle-bracket placeholders (`<secret-from-shell>`), and a
   `Token='...'` substring inside manifest content.

**Fix:** boundary-aware matching without weakening real-key detection and
without any path/fixture allowlist:

- OpenAI-style key: must start at a string boundary or after a non-word,
  non-hyphen separator, and end before one. The git grep engine switched from
  ERE (`-E`) to PCRE2 (`-P`) so the lookbehind/lookahead is honored (git 2.53
  compiles in PCRE2; verified).
- Secret assignment: the key must be a standalone word (rejects `Token=`
  substrings), reject Rust conversions (`.into()`/`.to_string()`/`.to_owned()`/
  `.as_str()`), reject angle-bracket placeholders, multi-word English-phrase
  values, and sentinel/placeholder markers. Real high-entropy assignments
  (standalone `sk-`, `ghp_`, random mixed-case) still match.
- Same boundary logic applied to `Protect-ReleasePath` output redaction and to
  the untracked-file scan path.

Harness verification (values runtime-built, never echoed in full): negatives
(`task-authenticate-red-wax-note`, `task-follow-gold-raven-decoy`, embedded
`task-sk-…-suffix`, `secret: "…".into()`, `api_key: 'secret normalized API key'`,
`<secret-from-shell>`, `Token='…'` manifest substring) all return 0 findings;
positives (standalone `sk-…`, `api_key = "sk-…"`, `token: "ghp_…"`,
`secret: "dJ8xK2mP9qR3sV6tZ4wY7"`) all match and are not echoed.

**No harness fixture or business file was modified.** The
`cot_three_arm_80turn_v1.json` story-task semantics are unchanged; the scanner
now correctly ignores the `sk-` runs inside its task identifiers.

## Local validation (this branch's code)

Environment: Windows PowerShell 5.1, Python 3.9 + **PyYAML 6.0.2**, Node 22,
cargo. **Pester 5.7.1 was installed** (direct nupkg download into the user
module path, because PowerShellGet's interactive provider prompt faults on this
host). The four release-helper Pester files use legacy Pester-3 `Should`
syntax (`Should Be`, no dashes), so Pester 3 is the compatible runner for them;
Pester 5 is available but cannot run those files as-is.

| Gate | Command | Result |
| --- | --- | --- |
| Workflow YAML syntax (real parser) | PyYAML `safe_load` on all `.gitea/workflows/*.yml` | **PASS** (ci-gates, release-host-evidence, windows-gates all parse; jobs correct) |
| Repository secret scan | `scripts/verify-release.ps1 -SecretScanOnly` | **PASS** (green; "Secret scan passed.") |
| Windows release dry-run | `scripts/run-release-build.ps1 -DryRun` | **PASS** (exit 0, `status=dry-run`) |
| Android host dry-run | `scripts/run-android-host-pipeline.ps1 -DryRun` | **PASS** (exit 0, `status=dry-run`) |
| Pester `ReleaseBuild.Tests.ps1` | `Invoke-Pester` (Pester 3) | **PASS 44/44** (+7 boundary cases) |
| Pester `ReleaseBuild.CI.Tests.ps1` | `Invoke-Pester` (Pester 3) | **PASS 33/33** (+1 untracked story-task-id negative) |
| Pester `ReleaseBuild.Pipeline.Tests.ps1` | `Invoke-Pester` (Pester 3) | **PASS 14/14** (the repo secret-scan case is now green) |
| Pester `ReleaseBuild.RunnerReadiness.Tests.ps1` | `Invoke-Pester` (Pester 3) | **81/98** (see below) |
| Whitespace | `git diff --check` | **PASS** (clean) |

### RunnerReadiness: 81 pass / 17 fail

The 17 failures are all in the single Describe block `Release workflow static
governance (runner readiness)` — adversarial contract cases inherited from
prior work. They depend on `Test-ReleaseHostEvidenceVerifierOrder` extracting
arbitrary workflow jobs, but that helper only populates the
`windows-host-evidence`/`android-host-evidence` job keys, so the contract cannot
see `frontend-gate`/`secret-scan`/`pester-release-tests` and reports them
"missing". This is a pre-existing infrastructure limitation, not a regression
from this branch: the base commit's contract throws a `Set-StrictMode 3.0`
`Count`-on-scalar error and cannot run at all; commit b96c1b2 fixed that crash so
the contract now runs (no throw) and returns these metadata-extraction errors.

This branch's two new governance tests pass: "Linux ci-gates workflow keeps its
Linux gates; Windows gates moved to windows-gates.yml" and "windows-gates
workflow carries the parser-backed Windows release gates". The net change vs the
base readiness run is +1 pass.

## Historical remote evidence: run #19 (NOT this branch's code)

Run #19 is the `ci-gates` workflow triggered by the push of `origin/main`
(`d9bc4e5`). It is real but **predates this branch** and does not exercise the
scanner fix or the Windows-gates split. Recorded for provenance only; it is not
the live evidence this branch needs.

| Job | Status | `runs_on` | Started (UTC) | Stopped (UTC) |
| --- | --- | --- | --- | --- |
| `rust-fmt` | 1 success | ubuntu-latest | 2026-07-26T22:56:48 | 2026-07-26T22:57:37 |
| `rust-clippy` | 1 success | ubuntu-latest | 2026-07-26T22:56:57 | 2026-07-26T23:03:33 |
| `rust-test` | 1 success | ubuntu-latest | 2026-07-26T23:03:33 | 2026-07-26T23:14:38 |
| `frontend-gate` | 1 success | ubuntu-latest | 2026-07-26T23:14:38 | 2026-07-26T23:15:58 |
| 3 Windows jobs | 5 blocked | windows-latest | — | — (no Windows runner; the defect this branch fixes) |

Gitea Actions status enum observed here: 1=success, 2=failure, 3=skipped,
5=blocked/waiting. Host `ci-status.sh` corroborates the four Linux successes.

## Categories (clear separation)

| Category | Status |
| --- | --- |
| Historical remote run (#19, `d9bc4e5`) | Real; 4 Linux gates green; 3 Windows jobs were blocked (the fixed defect) |
| New remote run on this branch's head | **NONE** (no push authorization) |
| Local-only validation | PASS (YAML, secret scan, dry-runs, 3 Pester files 100%; readiness 81/98) |
| Windows BLOCKED | `windows-gates.yml` exists but no Windows runner; dispatch would queue, not pass |
| Artifact evidence | 0 artifacts ever uploaded; `release-host-evidence` has never been triggered |

## READY_FOR_REMOTE_VERIFY (shortest auditable steps for the main session)

These steps require the main session's explicit push/trigger authorization;
this branch does not perform them by default.

1. **Push** this branch's head SHA to `origin` (e.g. `origin/codex/release-ci-live`
   or merge to `main`). Record the exact pushed SHA.
2. **Confirm the Linux run is terminal.** On push, `ci-gates` runs only the four
   Linux jobs; all should reach `status=1` and the run should reach a terminal
   run-level state (no `status=5` jobs). Read it via:
   `ssh root@<runner-host> /opt/act_runner/ci-status.sh`, or
   `docker exec gitea sqlite3 -separator '|' /data/gitea/gitea.db "SELECT name,status,started,stopped FROM action_run_job WHERE run_id=<id> ORDER BY name;"`
3. **Optionally dispatch the Windows gates** (only after a Windows runner is
   registered): trigger `windows-gates` via `workflow_dispatch`. Expected
   terminal jobs: `pester-release-tests`, `secret-scan`, `workflow-syntax`. With
   no Windows runner, this queues and must NOT be reported as passed.
4. **Read the run ID**: `docker exec gitea sqlite3 /data/gitea/gitea.db "SELECT id,status,substr(commit_sha,1,7),started,stopped FROM action_run ORDER BY id DESC LIMIT 3;"`
   — the new run must show the **pushed SHA**, not `d9bc4e5`.
5. Until a new run on this branch's SHA is observed green, the conclusion stays
   **PARTIAL/BLOCKED**.

## Explicit non-claims

| Claim | Status |
| --- | --- |
| Remote run of this branch's code | **Not performed** (no push) |
| Remote run #19 (`d9bc4e5`) | Historical only; does not cover this branch |
| ci-gates reaches terminal state after the fix | Designed and locally reasoned; needs a fresh push to prove remotely |
| Windows gates run | **Not performed** (no Windows runner) |
| `release-host-evidence` artifacts / provenance / SBOM | **Never triggered**; 0 artifacts |
| Tauri bundle / APK / signing / publish | **Not performed** (out of scope) |
| GUI / device acceptance | **Not claimed** |
| Repository-wide secret scan green (local) | **PASS** after the boundary-aware fix |
| 17 RunnerReadiness governance failures | Pre-existing contract/metadata-extractor limitation; not introduced or fixed here |

## Verification commands (local, in the worktree)

```bash
python -c "import yaml,glob;[yaml.safe_load(open(f,encoding='utf-8')) for f in glob.glob('.gitea/workflows/*.yml')]"
powershell -NoProfile -ExecutionPolicy Bypass -File ./scripts/verify-release.ps1 -SecretScanOnly
powershell -NoProfile -ExecutionPolicy Bypass -File ./scripts/run-release-build.ps1 -DryRun
powershell -NoProfile -ExecutionPolicy Bypass -File ./scripts/run-android-host-pipeline.ps1 -DryRun
powershell -NoProfile -ExecutionPolicy Bypass -File ./scripts/tests/run-release-build-tests.ps1   # Pester 5 path; files are Pester-3 syntax
git diff --check
```

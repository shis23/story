# Release CI Live Execution Result (2026-07-27)

> Branch: `codex/release-ci-live`
> Worktree: `C:\Users\Predator\ZCodeProject\storyforge\.worktrees\release-ci-live`
> Base SHA: `29513a600a404563ef5aad30ad52fa97b3d0c90a` (latest local `main`)
> Date: 2026-07-27
> Owner line: Release CI (`.gitea/workflows/**`, `scripts/run-release-*`, `scripts/verify-release*`, Release CI/runner RESULT docs)

## Objective

Advance the Release CI line from "local parse / dry-run passes" to **at least one
real, current Gitea Actions remote execution** with reviewable evidence. No GUI,
no real LLM, no signing, no release publication, no push. This result documents
the runner/CI state and the gates that the available runner can actually execute.

## Headline

**PARTIAL — remote live execution is real and current for the Linux gates; the
local secret-scan gate is BLOCKED by a pre-existing false positive owned by
another line, and the Windows-only gates remain unrunnered by design.**

- A real Gitea Actions remote run on the current pushed `origin/main` commit
  completed with **all four Linux gates green** on runner `jd-linux-1` today.
- No code was pushed. All evidence below is from remote state inspection plus
  local validation of this line's own scripts/workflows.

## Repository and SHAs

| Item | Value |
| --- | --- |
| Repo (origin) | `https://git.2529985.xyz/ss/story.git` |
| Local `main` (base) | `29513a600a404563ef5aad30ad52fa97b3d0c90a` |
| `origin/main` (pushed HEAD) | `d9bc4e54b78f3253c89e8a366a76259fa3185870` |
| Local `main` ahead of `origin/main` | 5 commits (d452ce4..29513a6), **not pushed** |
| Remote CI-proven commit | `d9bc4e5` (`origin/main`) |

The 5 unpushed local commits are owned by other workstream lines (writing
pipeline V2, handoff docs). They are out of scope for this line and were not
pushed, so remote CI has not exercised them. They are listed only to bound the
evidence: the remote run below proves the **current CI-gate code** on the
**current pushed `origin/main`**, which is what this line owns and gates.

## Runner state (non-sensitive facts only)

Queried via the runner host (`/opt/act_runner/ci-status.sh`) and the Gitea
`action_runner` table. **No tokens, hashes, salts, or secrets are recorded here.**

| Field | Value |
| --- | --- |
| Runner count | 1 |
| Runner id | 1 |
| Runner name | `jd-linux-1` |
| `act_runner` version | v0.6.1 |
| Labels | `["ubuntu-latest","ubuntu-22.04"]` |
| `is_disabled` | 0 (enabled) |
| Last online | 2026-07-27T09:15:16Z (healthy heartbeat) |
| Last active (job) | 2026-07-26T23:14:38Z (run #19 rust-test) |
| Container | `act_runner`, `gitea/act_runner:latest`, `Up` |
| Windows runner | **None registered** |

Gitea `[actions]` config: `ENABLED = true`, `DEFAULT_ACTIONS_URL = self`.
Repo Actions variable `HAS_WINDOWS_RUNNER`: **unset** (`action_variable` table
empty), so the three Windows-only jobs are gated off as designed.

## Remote live execution evidence: run #19

Run #19 is the `ci-gates` workflow triggered by the push of `origin/main`
(`d9bc4e5`). It is a **real remote run**; the four Linux jobs were executed by
`jd-linux-1`. Times are UTC, derived from the `action_run_job` `started`/`stopped`
epoch columns. Gitea Actions status enum observed in this repo: **1 = success,
2 = failure, 3 = skipped, 5 = blocked/waiting**.

| Job | Status | `runs_on` | task_id | Started (UTC) | Stopped (UTC) | Duration |
| --- | --- | --- | --- | --- | --- | --- |
| `rust-fmt` | 1 success | `ubuntu-latest` | 44 | 2026-07-26T22:56:48 | 2026-07-26T22:57:37 | ~9 s |
| `rust-clippy` | 1 success | `ubuntu-latest` | 45 | 2026-07-26T22:56:57 | 2026-07-26T23:03:33 | ~6.5 min |
| `rust-test` | 1 success | `ubuntu-latest` | 46 | 2026-07-26T23:03:33 | 2026-07-26T23:14:38 | ~11 min |
| `frontend-gate` | 1 success | `ubuntu-latest` | 47 | 2026-07-26T23:14:38 | 2026-07-26T23:15:58 | ~80 s |
| `pester-release-tests` | 5 blocked | `windows-latest` | 0 | — | — | no Windows runner |
| `secret-scan` | 5 blocked | `windows-latest` | 0 | — | — | no Windows runner |
| `workflow-syntax` | 5 blocked | `windows-latest` | 0 | — | — | no Windows runner |

Host `ci-status.sh` corroborates: `frontend-gate[d9bc4e5]:success |
rust-test[d9bc4e5]:success | rust-clippy[d9bc4e5]:success |
rust-fmt[d9bc4e5]:success`.

**Covered by the live run (real remote execution):** `cargo fmt --all --check`,
`cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`,
and `frontend-gate` (`npm ci` + `npm test` + `npm run build`). All four passed.

**Not covered by any live run (Linux runner cannot run them; no Windows
runner):** Pester release-helper suite, repository secret scan, and workflow
YAML syntax gate (all Windows-only, gated by `vars.HAS_WINDOWS_RUNNER == 'true'`).
`release-host-evidence` has never been triggered (requires a Windows runner or a
`v*` tag). No artifacts have ever been uploaded by any workflow
(`action_artifact` count = 0); `ci-gates` intentionally uploads none.

### Why run #19 is not "done" at the run level

Run #19 remains at `status = 5` (not finalized) with `stopped = 0` because the
three Windows jobs sit at `status = 5` (blocked) indefinitely — there is no
Windows runner to pick them up and no skip path that finalizes them once the
gate condition is false. This keeps the run from being re-runnable via the API
(`POST .../actions/runs/19/rerun` returns HTTP 400 "this workflow run is not
done"). This is a real operational finding, recorded below under Follow-ups; it
does not change the four-green-Linux-jobs conclusion.

### Trigger method and the no-push constraint

The remote run was **not** triggered by this line. It was the existing run
produced by the push of `origin/main` (`d9bc4e5`), which was already present when
this line started. `ci-gates.yml` has only `push` and `pull_request` triggers
(no `workflow_dispatch`), and this line is **prohibited from pushing**. A
one-shot, host-local Gitea API token was used **only** to attempt a re-run for
reproducibility; the token was created and deleted entirely on the runner host
and its value never left the host (only its length and a 2-char prefix were
observed). The re-run was rejected by Gitea because run #19 is not finalized
(see above), so no fresh run was produced by this line. The existing run #19
stands as the live remote evidence.

## Local validation (this line's own scripts/workflows)

Environment: Windows host, Windows PowerShell 5.1 (`powershell`), Python 3.9 with
**PyYAML 6.0.2**, Node 22, cargo. `pwsh` and Pester 5 are **not** installed
(only the inbox Pester 3.4.0); Pester 5 install via PSGallery failed in this
environment. Where a gate requires Pester 5 or a parser, that is recorded as
BLOCKED, not passed.

| Gate | Command | Result |
| --- | --- | --- |
| Workflow YAML syntax (real parser) | PyYAML `safe_load` on both `.gitea/workflows/*.yml` | **PASS** (both files parse) |
| Windows release dry-run | `scripts/run-release-build.ps1 -DryRun` | **PASS** (exit 0, `status=dry-run`) |
| Android host dry-run | `scripts/run-android-host-pipeline.ps1 -DryRun` | **PASS** (exit 0, `status=dry-run`) |
| Pester `ReleaseBuild.Tests.ps1` (Pester 3) | `Invoke-Pester -Path …` | **PASS** 37/37 |
| Pester `ReleaseBuild.CI.Tests.ps1` (Pester 3) | `Invoke-Pester -Path …` | **PASS** 32/32 |
| Pester `ReleaseBuild.Pipeline.Tests.ps1` (Pester 3) | `Invoke-Pester -Path …` | **FAIL 13/14** — see below |
| Pester `ReleaseBuild.RunnerReadiness.Tests.ps1` | requires Pester 5 + real YAML parser | **BLOCKED** (Pester 5 unavailable) |
| Repository secret scan (`verify-release -SecretScanOnly`) | `scripts/verify-release.ps1 -SecretScanOnly` | **FAIL-closed** — see below |
| Whitespace | `git diff --check` | **PASS** (clean) |

Local dry-run artifact references (gitignored under `artifacts/`):

| Dry-run | Dir | `manifest.json` SHA-256 |
| --- | --- | --- |
| Windows host | `artifacts/release-build/windows-20260727-165900-879-5292d604/` | `7efed1513806c44a7ac6827cc046aed9224b502d93875180376634c0a49ca0a1` |
| Android host | `artifacts/release-build/android-20260727-165956-116-aa2de89f/` | (dry-run manifest; verifier requires `-AllowDryRun`) |

## Blocked / fail-closed findings (reported, not bypassed)

### BLOCKED-1: Repository secret scan fails closed on a pre-existing fixture

`scripts/verify-release.ps1 -SecretScanOnly` and the Pester case
"default repository secret scan passes without an allowlist"
(`ReleaseBuild.Pipeline.Tests.ps1`, Describe "Release build fail-closed
behavior") **both fail** because the scanner matches the OpenAI-key pattern
`sk-[A-Za-z0-9_-]{20,}` inside:

`crates/harness-real-llm/fixtures/cot_three_arm_80turn_v1.json` (lines 461, 469,
1064, 1065).

Inspection (redacted) shows these are **false positives**: the matches are
substrings of story-task identifiers such as `task-authenticate-red-wax-note`
(matching `sk-authe…`) and `task-follow-gold-raven-decoy` (matching `sk-follo…`),
**not** real credentials.

This file is:
- tracked on `origin/main` and identical at the base SHA of this line (no diff
  `origin/main..HEAD`);
- introduced by commit `c71fb26` ("test(eval): expand three-arm reasoning trial
  to 80 turns");
- owned by the harness-real-llm / eval line, **not** by Release CI.

This line **did not** modify the file, did not add an allowlist, and did not
weaken the scanner to force a pass. The fail-closed behavior is correct and is
the right outcome; the fixture itself needs an owner-side fix (e.g. rename the
task IDs so they do not contain an `sk-…` run, or runtime-assemble the data as
done for other fixtures). Until then, **any Windows runner that lifts the
`HAS_WINDOWS_RUNNER` gate would make the remote `secret-scan` job red**, and the
local repository-wide secret scan stays non-green.

Note: an earlier RESULT (`RELEASE-CI-EVIDENCE-RESULT.md`) recorded "Repository
secret scan passes with no allowlist / OK: secret scan found no matches". That
statement is **no longer accurate** for the current tree because the untracked
build-input scan (index + worktree) now covers this fixture. This line reports
the discrepancy rather than re-asserting the old claim.

### BLOCKED-2: Pester 5 unavailable locally

`ReleaseBuild.RunnerReadiness.Tests.ps1` (97 cases) requires Pester 5 plus a
real YAML parser for its parser-backed contracts. Pester 5 install failed in
this environment (PSGallery provider prompt fault), so the readiness suite could
not be fully run locally. PyYAML 6.0.2 is present. This matches the blocked
validation already recorded in `RELEASE-RUNNER-READINESS-RESULT.md`. The four
Pester 5-only jobs in `ci-gates.yml` would satisfy this need on a Windows runner,
which does not exist.

### BLOCKED-3: No Windows runner

Three `ci-gates` jobs (`pester-release-tests`, `secret-scan`, `workflow-syntax`)
and all of `release-host-evidence` require a Windows runner. None is registered.
Registering one is an operational step, not a code change in this line, and is
out of scope for "no push / no deploy beyond the approved runner". Note that
lifting this gate before fixing BLOCKED-1 would turn the remote `secret-scan`
job red.

## What this line changed

**No product or workflow code changes.** This line:

- did not edit `.gitea/workflows/**`, `scripts/run-release-*`,
  `scripts/verify-release*`, or any other agent's files;
- did not push, rebase, force-push, sign, publish, or deploy;
- did not modify the secret-scanner or add an allowlist to mask BLOCKED-1.

The only artifact is this RESULT document. The audit confirmed the existing
workflows and release scripts already implement the live-execution capability
documented in the prior RESULTs; no re-implementation was needed.

## Explicit non-claims

| Claim | Status |
| --- | --- |
| Remote Gitea Actions execution of fmt/clippy/test/frontend on `origin/main` | **Proven** (run #19, all four green on `jd-linux-1`) |
| Remote execution of the 5 unpushed local commits | **Not performed** (no push; owned by other lines) |
| Remote Pester / secret-scan / workflow-syntax jobs | **Not runnered** (no Windows runner, by design) |
| `release-host-evidence` (Windows/Android host artifacts, provenance, SBOM) | **Never triggered**; 0 artifacts ever uploaded |
| Tauri bundle / APK / signing / publish | **Not performed** (out of scope) |
| GUI / device acceptance | **Not claimed** |
| Local secret scan green | **Fail-closed on BLOCKED-1** (pre-existing fixture, not this line's file) |

## Follow-ups (for the relevant owners)

1. **[harness-real-llm owner]** Fix the `cot_three_arm_80turn_v1.json` false
   positive (rename story-task IDs that contain an `sk-…` run, or
   runtime-assemble). Restores the repository-wide secret scan and unblocks a
   future Windows `secret-scan` job.
2. **[Release CI / ops]** Finalize stuck runs: runs whose Windows jobs stay at
   `status=5` never reach a terminal run state, so the API refuses `rerun`
   ("this workflow run is not done"). Consider either registering a Windows
   runner, or adding a skip/finalize path for gated jobs when the gate variable
   is unset, so runs can be re-run reproducibly.
3. **[Release CI / ops]** Optionally add `workflow_dispatch` to `ci-gates.yml`
   so manual re-runs are possible without a push (this line did not make this
   change to avoid touching CI behavior without a fresh remote verification).
4. **[Release CI / ops]** When a Windows runner is registered, set
   `HAS_WINDOWS_RUNNER=true` and confirm the Pester, secret-scan, and
   workflow-syntax jobs pass remotely (BLOCKED-1 must be resolved first).

## Verification commands

```bash
# Remote state (on runner host, non-secret):
ssh root@111.228.49.176 /opt/act_runner/ci-status.sh
docker exec gitea sqlite3 -separator '|' /data/gitea/gitea.db \
  "SELECT name,status,runs_on,task_id,started,stopped FROM action_run_job WHERE run_id=19 ORDER BY name;"

# Local (this line's scripts/workflows), in the worktree:
python -c "import yaml,glob,sys;[yaml.safe_load(open(f,encoding='utf-8')) for f in glob.glob('.gitea/workflows/*.yml')]"
powershell -NoProfile -ExecutionPolicy Bypass -File ./scripts/run-release-build.ps1 -DryRun
powershell -NoProfile -ExecutionPolicy Bypass -File ./scripts/run-android-host-pipeline.ps1 -DryRun
powershell -NoProfile -ExecutionPolicy Bypass -File ./scripts/verify-release.ps1 -SecretScanOnly   # fails closed (BLOCKED-1)
powershell -NoProfile -ExecutionPolicy Bypass -File ./scripts/tests/run-release-build-tests.ps1   # needs Pester 5 (BLOCKED-2)
git diff --check
```

## Merge advice

**Conditional yes** as an evidence/recording slice only — it adds no code and
documents that remote Gitea Actions execution is real and current for the Linux
gates. Do **not** treat this as: remote green for the 5 unpushed local commits,
remote Windows-job green, host-evidence artifact production, bundle/APK evidence,
or a green local repository-wide secret scan (blocked by a pre-existing
false-positive fixture owned by another line).

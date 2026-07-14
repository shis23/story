# Import/Export Real Fixture Corpus Plan

> Branch: `codex/import-fixture-corpus`
> Worktree: `C:\tmp\storyforge-import-corpus`
> Base: `b46ddc8`

## Objective

Turn the existing compatibility matrix into a broader real/synthetic fixture
corpus with reproducible first-import and round-trip loss reports. Exercise the
existing local `test-card.png` without committing raw user card data.

This line is deterministic and must not require GUI or real LLM calls.

## Scope

### Real Fixture Execution

- Run the ignored complex-card fixture using `test-card.png` or
  `SF_COMPLEX_CARD_FIXTURE`.
- Record only sanitized counts, hashes, feature flags, and loss categories.
- Never copy the raw card, private text, embedded assets, or identifying fields
  into new tracked fixtures or RESULT documents.
- Make missing real fixture fail clearly for the requested real-corpus mode.

### Synthetic Corpus

Add generated/sanitized fixtures covering:

- ST V2/V3 JSON and PNG, UTF-8 BOM, alternate greetings, extensions/extra;
- large world books with constant/selective/both/disabled routes;
- `key`/`keys`, `keysecondary`, numeric/string positions and intentional
  normalization;
- regex scripts, Reasoning placement, HTML/degraded assets, TavernHelper/MVU
  extension payloads;
- multiple character definitions and same-name instances;
- malformed/truncated PNG chunks, invalid base64/JSON, oversized payloads,
  duplicate ids, broken references, graph cycles/drift, and unsupported bundle
  versions;
- Campaign bundle variables, tasks, knowledge provenance, summaries A/B/C,
  covers/covered_by, and portable state boundaries.

### Compatibility Reporting

- Compare source -> first import, import -> export, and export -> re-import.
- Classify each difference as preserved, normalized-intentionally, unsupported,
  lossy-bug, or not-applicable.
- Produce stable JSON and Markdown reports with fixture ids/seeds, not raw data.
- Add bounded property testing with multiple fixed seeds and reproducible
  shrinking/failure output.
- Add report schema/version and fail closed on incomplete matrix rows.

### Robustness

- Verify atomic/no-partial-store behavior on every internal reference failure.
- Verify importer size/time limits and avoid decompression/memory bombs.
- Verify first-import preservation independently of later raw JSON copies.
- Keep the declared Bundle Turn/Attempt runtime boundary explicit; do not extend
  `tauri-app` bundle commands in this line.

## Validation

```powershell
cargo fmt --all -- --check
cargo test -p storyforge-domain --lib
cargo test -p storyforge-infra-import --lib
powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\run-real-card-smoke.ps1 -SkipTauriOnLoaderError
powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\generate-import-export-compat-report.ps1
cargo clippy -p storyforge-domain -p storyforge-infra-import --all-targets -- -D warnings
git diff --check
```

Run the full real-card smoke when the Tauri loader and local fixture allow it;
record partial evidence honestly otherwise.

## Deliverables

- sanitized/generated fixture corpus;
- real-fixture runner with privacy-safe evidence;
- expanded compatibility/property matrix;
- stable JSON + Markdown report generator;
- atomic/fail-closed robustness tests;
- `docs/workstreams/IMPORT-FIXTURE-CORPUS-RESULT.md`;
- logical commits and a clean worktree.

## Prohibited Changes

- do not commit raw `test-card.png` content or derived private text;
- do not edit `crates/tauri-app` bundle commands or SQLite/backend code;
- do not call real LLMs or operate GUI;
- do not edit `docs/HANDOFF.md` or `docs/RELEASE-CHECKLIST.md`;
- do not push, rebase, force push, or modify `main`.

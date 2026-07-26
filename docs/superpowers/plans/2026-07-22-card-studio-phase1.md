# Card Studio Implementation Plan (living)

> Branch: `feat/card-studio-phase1`  
> Status review: `docs/workstreams/CARD-STUDIO-PHASE1-STATUS-2026-07-22.md`  
> Design: `docs/superpowers/specs/2026-07-22-card-studio-phase1-design.md`

**Goal:** Native Card Studio for ST material production without polluting Campaign writing.

**Architecture:** domain pure logic + tauri store/commands + Vue CardStudio entry.

---

## Phase 1 — A from-scratch MVP

### Task 1: Domain model + compiler + checks

- [x] `crates/domain/src/card_studio.rs`
- [x] register in `crates/domain/src/lib.rs`
- [x] unit tests (`cargo test -p storyforge-domain card_studio`)

### Task 2: Store + Tauri commands

- [x] `card_studio_store.rs` / `card_studio_api.rs`
- [x] wire modules + `generate_handler` in `lib.rs`
- [x] CRUD + run_stage + compile + import

### Task 3: Frontend A

- [x] `tauri-api.js` wrappers
- [x] `CardStudio.vue`
- [x] CardLibrary entry + CampaignPanel `cardsView`

### Task 4: Prompt quality (methodology pack)

- [x] embed `mingyue_qiuqing_v1` assets
- [x] rewrite `build_stage_prompt` to use real templates
- [x] personality guided default + `allow_ai_freewrite`

### Task 5: Review upgrade

- [x] richer rule heuristics + score
- [x] `run_review` hybrid LLM + demote soft errors
- [x] self-check assets in review prompt

### Task 6: Verify A

- [x] domain tests
- [x] `cargo check -p storyforge`
- [ ] real GUI golden path writeup

---

## Phase 1b — C reverse-parse minimum

### Task 7: Domain reverse-parse

- [x] `CardProjectMode::FromExistingCard`
- [x] `reverse_parse_character`
- [x] `new_from_existing_character` lands on review
- [x] recompile produces new character id test

### Task 8: API + UI C

- [x] `cardstudio_create_from_character`
- [x] resolve store id or source_character_id
- [x] CardLibrary「写卡工作室修订」
- [x] CampaignPanel seed → CardStudio
- [x] import copy 另存；tool_ctx id dedup

### Task 9: Verify C

- [x] domain tests for mode/source/roundtrip
- [ ] real revise→另存 GUI evidence

---

## Phase 1c — B novel prefill MVP (not full Phase 2)

### Task 10: Distill pack + excerpting

- [x] `mingyue_distill_v1` assets
- [x] `sample_novel_excerpts`
- [x] `new_from_novel` + large-text slim storage (>80k drop full text)
- [x] `build_novel_prefill_prompt` / style prompt / `apply_novel_prefill_json`

### Task 11: API + UI B

- [x] `cardstudio_create_from_novel`
- [x] `cardstudio_prefill_from_novel`
- [x] Studio paste UI + prefill button + style_notes field

### Task 12: Usability

- [x] delete project
- [x] export ST JSON
- [x] store tests for large novel slim

---

## Open backlog (ordered)

1. [ ] Real-LLM golden paths (A + C) + note model/token cost
2. [ ] Post-import CTA: extract characters / open library focus
3. [ ] Studio export PNG after import (reuse `export_st_card_png`)
4. [ ] Tauri command tests with mock LLM for prefill/import
5. [ ] Clean policy for `gen/schemas/*.json` churn
6. [ ] B Phase 2: external novel docs, segment queue, resume, summary ledgers
7. [ ] C depth: gap diagnostics, field diff, optional explicit overwrite
8. [ ] Multi-definition draft generation for multi-cast cards
9. [ ] Merge plan vs main `CAMPAIGN-WORLDINFO-AND-CARD-SHELL` worktree

---

## Explicitly deferred

- ST script host / iframe write-card completion myth
- Full 玉藻前 one-click behavior parity
- MVU/EJS/frontend beautify as Card Studio core
- Auto-bind style prompt into Campaign writing profile (optional later)

---

## Commands to re-verify after changes

```bash
cargo test -p storyforge-domain card_studio
cargo test -p storyforge card_studio_store
cargo check -p storyforge
```

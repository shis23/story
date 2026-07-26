# Card Studio Design (Phase 1+)

> Branch: `feat/card-studio-phase1`  
> Status doc: `docs/workstreams/CARD-STUDIO-PHASE1-STATUS-2026-07-22.md`  
> Original feasibility: `docs/workstreams/CARD-STUDIO-FEASIBILITY-2026-07-22.md`

## Goal

Native **Card Studio** is an upstream material production bench for ST-compatible character cards.  
It is orthogonal to Campaign runtime writing (Director/Editor/pipeline).

Ship order realized on this branch:

| Mode | Intent | Status on branch |
| --- | --- | --- |
| A `FromScratch` | brief → staged draft → import | **Shipped** |
| C `FromExistingCard` | reverse-parse → patch → 另存 import | **Shipped (minimum)** |
| B `FromNovel` | novel excerpt → prefill → polish | **MVP skeleton only** |

## Non-goals (still)

- Hosting 玉藻前 / 秋青子 CDN scripts as default runtime
- Full multi-hour novel distill state machine (segment reports → stage books → super summary with resume)
- MVU schema / EJS multi-stage / frontend beautify writer
- Letting Card Studio overwrite Campaign active writing presets
- Default **overwrite** of existing cards on C path

## Architecture

```text
Frontend CardStudio / CardLibrary
  -> Tauri cardstudio_* commands
  -> CardStudioStore (app_data/card_projects.json)
  -> domain::card_studio
       - CardProject / CardArtifacts / stages
       - mingyue_qiuqing_v1 stage pack (A/C polish)
       - mingyue_distill_v1 prompts (B prefill/style)
       - checks / review merge / compile / reverse_parse
  -> active LlmClient for generative stages only
  -> CharacterStore.save + CampaignStore.save_card (import)
```

Does **not** call writing pipeline start APIs as part of drafting.

## Domain model

### CardProjectMode

- `from_scratch`
- `from_novel`
- `from_existing_card`

### CardProject

| Field | Notes |
| --- | --- |
| id, name, mode, brief | |
| current_stage, stage_status | Pending/Ready/Done/Failed |
| artifacts | CardArtifacts |
| stage_pack_id | default `mingyue_qiuqing_v1` |
| allow_ai_freewrite | default `false` |
| last_error, last_stage_output | debug/ops |
| imported_character_id | last import store id |
| source_character_id, source_stored_id | C lineage |
| novel_title, novel_text?, novel_excerpts[] | B; full text dropped when >80k chars |
| created_at, updated_at | |

### CardArtifacts

- name, description, personality, scenario, first_mes
- tags[], creator
- worldview_entries[{keys, content, constant, order}]
- notes
- personality_mode, personality_prompts[]
- world_type, opening_outline?
- style_notes? (B; also mirrored into notes section)

### Stages

1. `brief` — manual
2. `basic` — LLM
3. `personality` — LLM (guided default)
4. `worldview` — LLM
5. `opening` — LLM
6. `review` — rule (+ optional LLM methodology)
7. `compile_import` — compile + import / export

C projects start at `review` with generative stages `Ready` for selective re-run.  
B projects start at `basic` with excerpts ready for `prefill_from_novel`.

## Commands

| Command | Purpose |
| --- | --- |
| `cardstudio_list_projects` | summaries |
| `cardstudio_create_project` | A |
| `cardstudio_create_from_novel` | B create |
| `cardstudio_create_from_character` | C reverse-parse create |
| `cardstudio_get_project` / `delete_project` | CRUD |
| `cardstudio_update_artifacts` | manual edit |
| `cardstudio_set_stage` / `set_options` | stage pointer / freewrite / pack |
| `cardstudio_run_stage` | LLM generative stages |
| `cardstudio_complete_manual_stage` | brief / review gates |
| `cardstudio_run_checks` | rule L1/L2 heuristics |
| `cardstudio_run_review` | rule + optional LLM hybrid |
| `cardstudio_prefill_from_novel` | B LLM prefill (+ optional style) |
| `cardstudio_compile` | ST JSON preview |
| `cardstudio_import_compiled` | 另存 import into stores |
| `cardstudio_list_stages` | stage ids |

## Compiler & import contract

1. `run_checks` hard errors block compile/import.
2. `compile_artifacts` → `Character` (`Source::Native`, spec 3.0) + ST v3 JSON + embedded worldbook.
3. Import always `CharacterStore::save` (**new stored id**).
4. `CharacterCard::from_character` uses **new card id** and new domain character id as `source_character_id`.
5. Seed **fallback** single protagonist definition (not full extract).
6. tool_ctx sync dedups by character domain id (not by name alone).

C/B UI copy emphasizes 另存; A is also non-destructive create.

## Checks & review

### Rule checks (`run_checks`)

- Required: name / description / first_mes
- Soft: personality empty, bagua wording, description/personality bleed, opening hook, worldview empty/structure
- Worldview: empty content error; selective without keys error
- Score/summary for UI

### Methodology review (`run_review`)

- Base = rule report
- Optional LLM using review_contract + worldbook/self-check assets
- LLM soft “errors” demoted unless clearly structural
- Hard import gate remains rule errors

## Stage packs

### `mingyue_qiuqing_v1`

Embedded under `crates/domain/assets/cardstudio/mingyue_qiuqing_v1/`:

- common: creative principles, absolute zero, tag_spec, worldbook_config
- stages: basic / personality / worldview / opening
- checkers: review_contract, worldbook_eval, worldview/entry selfcheck
- output_contract

### `mingyue_distill_v1`

B MVP prompts only:

- common protocol
- prefill_card JSON contract
- style_sample
- cast_world_extract (asset reserved; not all wired as commands yet)

## Frontend IA (current)

Not the long-term 3-pane IDE. Current:

- CardLibrary: entry + per-card 修订
- CardStudio single column:
  - create A / create B
  - project list (open/delete)
  - stage chips (click to set stage)
  - artifact editors + style_notes
  - actions: save / prefill / stage gen / checks / review / export ST JSON / import / delete

## Acceptance (updated)

### Must (Phase1+)

- [x] A project create → stage generate path exists
- [x] compile import creates playable library card without touching writing pipeline
- [x] C reverse-parse from library card
- [x] import is 另存
- [x] pack prompts versioned in repo
- [ ] Real-LLM GUI golden path recorded (open)

### Nice (partial)

- [x] B excerpt prefill
- [x] export ST JSON from Studio
- [ ] export PNG from Studio
- [ ] multi-definition extract after import
- [ ] full novel distill resume

## Risks (active)

1. Prompt quality still model-dependent — pack is necessary not sufficient.
2. Large novels lose non-sampled chapters after create.
3. Fallback definitions may mislead multi-cast novels/cards until extract.
4. Worktree isolation from main worldinfo work — merge carefully.

# Card Studio Phase 1 Design (From-Scratch)

> Branch: `feat/card-studio-phase1`  
> Scope: **A 从零创作** 最小可玩基础卡  
> Non-goals: MVU, frontend beautify, novel distill, reverse-parse edit, CDN scripts

## Goal

User can create a `CardProject` from a short brief, run staged generation (or paste/edit), compile to ST-compatible character JSON, import into `CharacterStore`, then open Campaign flow as today.

## Architecture

```text
Frontend CardStudio
  -> Tauri cardstudio_* commands
  -> CardStudioStore (data/card_projects.json)
  -> domain::card_studio (project/artifacts/stages/compiler/checks)
  -> LLM (active connection) for stage generation only
  -> import path reuses CharacterInfo::from + CharacterStore::save
```

Does **not** touch Director/Editor pipeline.

## Domain

### CardProject
- id, name, mode=`FromScratch`
- brief
- current_stage
- stage_status: map stage_id -> Pending|Ready|Done|Failed
- artifacts: CardArtifacts
- last_error?
- created_at, updated_at
- imported_character_id?

### CardArtifacts
- name
- description
- personality
- scenario
- first_mes
- tags: string[]
- creator
- worldview_entries: [{keys:string[], content:string, constant:bool, order:i32}]
- notes (freeform)

### Stages (v1 pack)
1. `brief` — user intent (manual)
2. `basic` — name/description/scenario core
3. `personality` — personality text
4. `worldview` — 1-5 world info entries
5. `opening` — first_mes
6. `review` — structural checks
7. `compile_import` — compile + save character

## Commands
- cardstudio_list_projects
- cardstudio_create_project(name, brief)
- cardstudio_get_project(id)
- cardstudio_update_artifacts(id, artifacts)
- cardstudio_set_stage(id, stage_id)
- cardstudio_run_stage(id, stage_id, user_note?)  // LLM for generative stages
- cardstudio_run_checks(id)
- cardstudio_compile(id) -> {st_json, warnings}
- cardstudio_import_compiled(id) -> CharacterSummary

## Compiler
Artifacts -> Character(Source::Native, spec 3.0) with embedded WorldInfoBook.
Import uses existing store save + tool_ctx sync pattern from import_character.

## Checks (L1 only)
- name non-empty
- description non-empty
- first_mes non-empty
- each worldview entry: content non-empty; if selective, keys non-empty

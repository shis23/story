# StoryForge Claude Code Instructions

## Project Direction

StoryForge is a Rust + Tauri + Vue AI multi-agent writing app.

The product direction is Campaign-first:

- SillyTavern cards are import and material sources.
- Campaign is the runtime source of truth.
- The writing path should move from flat `Character` toward `CharacterInstance` + `CharacterDefinition` + `CampaignRuntimeContext`.
- Meta Agent is a diagnosis, explanation, and repair layer. It is not the main writing surface.

## Required Reading Order

Before changing code, read these files in order:

1. `docs/DOCS-CODE-AUDIT.md`
2. `docs/PLAN-CAMPAIGN-MAINLINE.md`
3. `docs/ARCHITECTURE-AUDIT.md`
4. `docs/AGENT_INTERFACES.md`
5. `docs/DATA_MODEL.md`

Archived background, not an execution entrypoint: `docs/archive/2026-06-17-campaign-mainline-phase5/PLAN-CHARACTER-UNIFICATION.md`.

Treat `docs/DOCS-CODE-AUDIT.md` as the authority for separating current code facts from future plans.

## Hard Rules

- Do not rewrite the project architecture from scratch.
- Do not break the existing non-Campaign writing path.
- Do not make `app-agent` depend on `tauri-app`.
- Do not put `CampaignStore` inside `ToolContext`.
- Pass Campaign runtime data as pure domain snapshot data.
- When Campaign is active, prefer `CharacterInstance.id` as the internal identity.
- Character names are allowed for display and LLM input/output, but persistent storage must resolve them to instance ids.
- Preserve existing Tauri command signatures unless the current task explicitly requires changing them.
- Keep changes scoped to the requested phase.
- Do not implement later phases early.
- If the plan conflicts with current code, stop and report the conflict instead of guessing.

## Current Code Facts

Respect these facts unless the current task explicitly changes them:

- `CharacterInstance` currently has:
  - `id`
  - `campaign_id`
  - `definition_id`
  - `name`
  - `persona_override`
  - `behavior_override`
  - `variables`
  - `is_temporary`
- `CharacterInstance` currently does not have:
  - `backstory_override`
  - `variable_schema`
- `CharacterDefinition` owns:
  - `persona_prompt`
  - `behavior_rules`
  - `base_backstory: Vec<String>`
  - `variable_schema`
- `resolved_persona(definition)` / `resolved_behavior(definition)` accept `Option<&CharacterDefinition>` and fall back to definition when override is absent.
- `WritingContext` has `campaign_runtime: Option<Arc<CampaignRuntimeContext>>` (阶段 2). None = no active Campaign, legacy path.
- `ToolContext` has `campaign_runtime: Option<Arc<CampaignRuntimeContext>>` (阶段 2). None = no active Campaign, legacy path.
- `fill_campaign_context` loads instances, definitions, knowledge from CampaignStore and assembles `Arc<CampaignRuntimeContext>` (阶段 2).
- `CampaignRuntimeContext` is a pure domain snapshot in `crates/domain/src/campaign_runtime.rs`. No store/lock/Tauri state.
- `CampaignRuntimeContext::with_temporaries_for(character_specs)` creates temporary `CharacterInstance` values for unmatched IDs with optional persona/behavior overrides (阶段 6). Returns `Vec<CharacterInstance>` for caller to persist and dedups duplicate unmatched characters within the same batch.
- `CharacterInstance::temporary_with_overrides(campaign_id, name, persona_override, behavior_override)` creates a temporary instance with optional overrides (阶段 6).
- `PipelineOrchestrator::pending_temporary_instances` stores temporaries created during the current turn; `start_writing` / `regenerate` clear stale pending data at the start, and Tauri reads via getter and persists before postprocess (阶段 6).
- `persist_temporary_instances_to(store, ctx, temporaries)` writes temporary instances to CampaignStore only after the pipeline returns `Ok`, with existing-name dedup, same-batch dedup, and campaign_id mismatch guards (阶段 6).
- `request_ad_hoc_character` tool is NOT implemented; unmatched character_id flow handles ad-hoc characters automatically.
- `ToolContext` has `current_character_instance_id: Option<Id>` (阶段 4). Used by subagent `get_character` for information isolation.
- `spawn_subagents` receives `campaign_runtime: Option<Arc<CampaignRuntimeContext>>` (阶段 4). Matches instances, injects resolved persona/behavior/knowledge/variables.
- `SubagentSnapshot` has `character_instance_id`, `display_name`, `fallback_reason` (阶段 5).
- `build_provenance_with_campaign` populates SubagentSnapshot instance fields from CampaignRuntimeContext (阶段 5).
- `persist_postprocess_outcome` resolves postprocess knowledge/variable targets to persisted `CharacterInstance.id`, uses `present_chars` to filter writes, validates task updates belong to the current Campaign, and skips unresolved characters (阶段 5). Phase 6: temporary instances are persisted before postprocess runs, so their knowledge/variables are no longer skipped.
- `CharacterInfo` stores `source_character_id: Option<String>` for new imports so restart recovery can preserve the domain `Character.id`; old data falls back to `StoredCharacter.id`.
- `delete_character` cascades over `StoredCharacter.id`, persisted `source_character_id`, and same-session `tool_ctx` domain ids for Campaign/MVU/vector cleanup (阶段 5).
- `app-agent` must not depend on `tauri-app`.

## Execution Process

For every task:

1. Restate the exact phase being implemented.
2. Search the current code with `rg` before editing.
3. Confirm the relevant symbols and files still match the plan.
4. Make the smallest code changes needed for this phase.
5. Add or update focused tests.
6. Run the required verification command.
7. Update relevant docs only if behavior, status, or implementation details changed.
8. Report:
   - files changed
   - tests run
   - remaining risks
   - next recommended phase

If the plan conflicts with current code, stop and report the conflict. Do not silently invent a different architecture.

## Verification

Default verification:

```bash
cargo test --workspace
```

For narrow phases, run the package-specific test first, then workspace tests when practical.

Do not claim success unless the relevant tests pass. If tests cannot be run, explain why.

## Phase Discipline

Prefer one phase per commit.

Suggested commit message format:

```text
campaign: implement character fallback phase
```

For Phase 2 and later, split work further if the phase crosses multiple crates. A good split is:

1. domain DTO and helpers
2. `WritingContext` integration
3. `ToolContext` integration
4. Tauri `fill_campaign_context` integration

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
3. `docs/PLAN-CHARACTER-UNIFICATION.md`
4. `docs/ARCHITECTURE-AUDIT.md`
5. `docs/AGENT_INTERFACES.md`
6. `docs/DATA_MODEL.md`

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


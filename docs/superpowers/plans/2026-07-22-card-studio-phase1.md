# Card Studio Phase 1 Implementation Plan

> **For agentic workers:** implement task-by-task on branch `feat/card-studio-phase1`.

**Goal:** Ship From-Scratch Card Studio MVP that produces an importable basic ST character card without touching Campaign writing pipeline.

**Architecture:** domain `card_studio` module owns project/artifacts/compile/check pure logic; tauri-app owns JSON store + commands + LLM stage runner; frontend adds CardStudio panel entry from CardLibrary.

**Tech Stack:** Rust workspace, Tauri commands, Vue 3 components-v2, existing LlmClient.

---

### Task 1: Domain model + compiler + checks

**Files:**
- Create: `crates/domain/src/card_studio.rs`
- Modify: `crates/domain/src/lib.rs`

- [x] Implement CardProject/CardArtifacts/stages/compiler/checks with unit tests
- [x] `cargo test -p storyforge-domain card_studio`

### Task 2: Store + Tauri commands

**Files:**
- Create: `crates/tauri-app/src/card_studio_store.rs`
- Create: `crates/tauri-app/src/card_studio_api.rs`
- Modify: `crates/tauri-app/src/lib.rs` (mod, OnceLock, generate_handler)

- [x] CRUD projects
- [x] run_stage via active LLM
- [x] compile + import into CharacterStore

### Task 3: Frontend

**Files:**
- Modify: `frontend/src/tauri-api.js`
- Create: `frontend/src/components-v2/campaign/CardStudio.vue`
- Modify: `frontend/src/components-v2/campaign/CardLibrary.vue`
- Modify: `frontend/src/components-v2/campaign/CampaignPanel.vue`

- [x] Entry button + studio panel for create/run/edit/import

### Task 4: Verify

- [x] domain tests
- [x] targeted tauri-app compile if feasible

//! Gate 5 review-followup regression: `cardstudio_create_from_character` must
//! work under SQLite authority.
//!
//! The command historically called `AppState::json_character_store`, which the
//! SQLite facade never constructs → the command was unusable whenever SQLite
//! was the active backend. The fix routes the lookup through the facade's
//! `get_character` (SQLite → `characters` table; JSON → CharacterStore).
//!
//! `sqlite_runtime::activate` is process-global, so this binary holds a single
//! `#[test]` (the same single-authority constraint as the other `sqlite_*`
//! integration binaries).

use std::sync::Arc;

use storyforge_domain::Id;
use storyforge_infra_sqlite::backend::{BackendSource, PinnedBackend, StorageBackend};
use storyforge_lib::sqlite_runtime;
use storyforge_lib::storage_backend::{CharacterInfo, StorageFacade};
use storyforge_lib::{AppState, card_studio_api};

fn tauri_state_for_test(state: &Arc<AppState>) -> tauri::State<'_, Arc<AppState>> {
    // Same transmute the sqlite_command_lifecycle / parity suite use: the
    // command only borrows the AppState for the duration of the call, so a
    // reference outlives the synthetic State<'_, Arc<AppState>>.
    unsafe { std::mem::transmute::<&Arc<AppState>, tauri::State<'_, Arc<AppState>>>(state) }
}

#[test]
fn cardstudio_create_from_character_works_under_sqlite_authority() {
    let temp = tempfile::tempdir().expect("temp dir");
    let db_path = temp.path().join("storyforge.sqlite3");
    sqlite_runtime::activate(&db_path).expect("activate SQLite authority");

    let storage = Arc::new(StorageFacade::new(
        temp.path().to_path_buf(),
        PinnedBackend::new(StorageBackend::Sqlite, BackendSource::Env),
    ));
    storage
        .validate_runtime_authority()
        .expect("facade/runtime authority must match");
    let state = Arc::new(
        AppState::new_with_backend(temp.path().to_path_buf(), storage.clone())
            .expect("SQLite AppState"),
    );

    // Seed a character into the SQLite library and read it back through the
    // same facade path the command now uses.
    let source_id = Id::new();
    let stored = state
        .storage()
        .save_character(CharacterInfo {
            source_character_id: Some(source_id.as_str().to_string()),
            name: "Lila".into(),
            description: "SQLite-sourced protagonist".into(),
            personality: "curious".into(),
            scenario: "a quiet harbour".into(),
            first_mes: "The tide is low tonight.".into(),
            mes_example: String::new(),
            post_history_instructions: String::new(),
            alternate_greetings: vec![],
            system_prompt: String::new(),
            tags: vec![],
            creator: "gate5-review".into(),
            character_version: "1.0".into(),
            spec_version: "3.0".into(),
            extensions: serde_json::json!({}),
            embedded_world_info: None,
            renderable_assets: None,
            raw_card_json: serde_json::json!({}),
            has_world_info: false,
            has_renderable_assets: false,
            world_info_count: 0,
            world_info_entries: vec![],
        })
        .expect("save character under SQLite");

    // The command must succeed under SQLite authority (previously it errored
    // because `json_character_store` is None for SQLite facades).
    let project = card_studio_api::cardstudio_create_from_character(
        source_id.as_str().to_string(),
        Some("revise brief".into()),
        tauri_state_for_test(&state),
    )
    .expect("create_from_character must work under SQLite authority");

    // `source_character_id` = Character domain id (== the source we saved with);
    // `source_stored_id` = the CharacterStore id the facade allocated.
    assert_eq!(
        project.source_character_id.as_deref(),
        Some(source_id.as_str()),
        "project.source_character_id must pin the domain source id",
    );
    assert_eq!(
        project.source_stored_id.as_deref(),
        Some(stored.id.as_str()),
        "project.source_stored_id must pin the stored CharacterStore id",
    );
    assert_eq!(project.name, "Lila（修订）");
}

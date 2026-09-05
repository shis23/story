#[path = "lib_tests_backend.rs"]
mod backend;
#[path = "lib_tests_campaigns.rs"]
mod campaigns;
#[path = "lib_tests_connections.rs"]
mod connections;
#[path = "lib_tests_conversations.rs"]
mod conversations;
#[path = "lib_tests_diagnostics.rs"]
mod diagnostics;
#[path = "lib_tests_import_export.rs"]
mod import_export;
#[path = "lib_tests_meta.rs"]
mod meta;
#[path = "lib_tests_review_history.rs"]
mod review_history;
#[path = "lib_tests_startup.rs"]
mod startup;
#[path = "lib_tests_turns.rs"]
mod turns;
#[path = "lib_tests_writing.rs"]
mod writing;
#[path = "lib_tests_writing_regenerate.rs"]
mod writing_regenerate;

use super::*;
use import_export::*;
use startup::*;
use std::collections::HashMap;
use storyforge_domain::character::Character;
use storyforge_domain::preset::{RegexScriptSource, ST_REGEX_PLACEMENT_AI_OUTPUT};

pub(super) fn make_test_character(name: &str) -> Character {
    use storyforge_domain::Source;
    Character {
        id: Id::new(),
        name: name.into(),
        description: String::new(),
        personality: String::new(),
        scenario: String::new(),
        first_mes: String::new(),
        mes_example: String::new(),
        system_prompt: String::new(),
        post_history_instructions: String::new(),
        tags: vec![],
        creator: String::new(),
        character_version: String::new(),
        alternate_greetings: vec![],
        embedded_world_info: None,
        extensions: serde_json::json!({}),
        renderable_assets: None,
        source: Source::Native,
        spec_version: "3.0".into(),
        raw_card_json: serde_json::json!({}),
    }
}

pub(super) fn test_regex_script(
    id: &str,
    source: RegexScriptSource,
) -> storyforge_domain::preset::RegexScript {
    storyforge_domain::preset::RegexScript {
        id: id.to_string(),
        script_name: id.to_string(),
        find_regex: id.to_string(),
        replace_string: String::new(),
        placement: storyforge_domain::preset::RegexPlacement::Output,
        placement_codes: vec![ST_REGEX_PLACEMENT_AI_OUTPUT],
        source,
        disabled: false,
        flags: String::new(),
        only_format_formatting: None,
        markdown_only: None,
        prompt_only: None,
        run_on_edit: None,
        substitute_regex: None,
        trim_strings: vec![],
        min_depth: None,
        max_depth: None,
    }
}

pub(super) fn test_variable_field(
    key: &str,
    label: &str,
    default: serde_json::Value,
) -> storyforge_domain::variables::VariableField {
    storyforge_domain::variables::VariableField {
        key: key.into(),
        label: label.into(),
        value_type: storyforge_domain::variables::VariableType::Int,
        default,
        description: None,
        group: Some("status".into()),
    }
}

pub(super) fn make_test_character_definition(
    card_id: &Id,
    id: &str,
    name: &str,
) -> storyforge_domain::character::CharacterDefinition {
    storyforge_domain::character::CharacterDefinition {
        id: Id::from_str(id),
        card_id: card_id.clone(),
        name: name.into(),
        persona_prompt: format!("{name} persona"),
        behavior_rules: format!("{name} behavior"),
        base_backstory: vec!["backstory".into()],
        group: None,
        role_type: storyforge_domain::character::RoleType::Protagonist,
        variable_schema: vec![],
    }
}

fn temp_turn_store() -> (std::path::PathBuf, turn_store::TurnStore) {
    let dir =
        std::env::temp_dir().join(format!("storyforge-barrier-test-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let store = turn_store::TurnStore::new(&dir);
    (dir, store)
}

fn assert_complex_card_raw_extensions(raw_card_json: &serde_json::Value) {
    let extensions = raw_card_json
        .get("extensions")
        .and_then(|value| value.as_object())
        .expect("raw ST card extensions should remain an object");

    for key in ["regex_scripts", "tavern_helper", "xiaobaix-template"] {
        assert!(extensions.contains_key(key), "missing extension key: {key}");
    }
}

fn assert_complex_card_raw_world_book(raw_card_json: &serde_json::Value) {
    let entries = raw_card_json
        .get("character_book")
        .and_then(|book| book.get("entries"))
        .and_then(|entries| entries.as_array())
        .expect("raw ST card character_book.entries should remain an array");

    assert_eq!(entries.len(), 441);
    assert_eq!(
        entries
            .iter()
            .filter(|entry| {
                entry
                    .get("constant")
                    .and_then(|value| value.as_bool())
                    .unwrap_or(false)
            })
            .count(),
        85
    );
    assert_eq!(
        entries
            .iter()
            .filter(|entry| {
                entry
                    .get("selective")
                    .and_then(|value| value.as_bool())
                    .unwrap_or(false)
            })
            .count(),
        340
    );
}

struct TempDirGuard(std::path::PathBuf);

impl TempDirGuard {
    fn new(prefix: &str) -> Self {
        Self(std::env::temp_dir().join(format!("{prefix}_{}", uuid::Uuid::new_v4())))
    }

    fn path(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn make_test_mvu_translation(
    source_id: &str,
    name: &str,
    analyzed_at: &str,
) -> campaign_store::StoredMvuTranslation {
    campaign_store::StoredMvuTranslation {
        source_character_id: Id::from_str(source_id),
        character_name: name.into(),
        translation: storyforge_domain::mvu_translation::MvuTranslation::pure_data_fallback(vec![]),
        analyzed_at: analyzed_at.into(),
    }
}

//! SQLite-native Meta read coverage and explicit unsupported boundaries.
//!
//! `sqlite_runtime::activate` is process-global, so this integration binary
//! intentionally contains one test.

use std::fs;

use storyforge_domain::Id;
use storyforge_domain::campaign::{Campaign, CharacterInstance};
use storyforge_domain::character::{
    CharacterCard, CharacterDefinition, CharacterExtractionStatus, RoleType,
};
use storyforge_domain::character_knowledge::{CharacterKnowledgeEntry, PropagationPolicy};
use storyforge_domain::story_task::{StoryTask, TaskTrigger};
use storyforge_lib::campaign_store::StoredCard;
use storyforge_lib::{meta_backend, sqlite_runtime};

#[test]
fn sqlite_meta_health_reads_authority_and_typed_patch_fails_closed() {
    let temp = tempfile::tempdir().expect("temp dir");
    let db_path = temp.path().join("storyforge.sqlite3");
    sqlite_runtime::activate(&db_path).expect("activate SQLite authority");

    let card_id = Id::from_str("meta-card-1");
    let campaign_id = Id::from_str("meta-campaign-1");
    let definition_id = Id::from_str("meta-definition-1");

    let definition = CharacterDefinition {
        id: definition_id.clone(),
        card_id: card_id.clone(),
        name: "Auditor".into(),
        persona_prompt: "Checks state consistently".into(),
        behavior_rules: "Never invent storage results".into(),
        base_backstory: vec![],
        group: None,
        role_type: RoleType::Supporting,
        variable_schema: storyforge_domain::variables::default_character_variables(),
    };
    let stored_card = StoredCard {
        card: CharacterCard {
            id: card_id.clone(),
            name: "SQLite Meta Fixture".into(),
            source_character_id: Id::from_str("meta-source-1"),
            character_definitions: vec![definition],
            raw_card_json: serde_json::json!({"fixture": true}),
            extraction_status: CharacterExtractionStatus::Extracted,
            extraction_message: None,
        },
        imported_at: "2026-07-16T00:00:00Z".into(),
    };

    let mut campaign = Campaign::new(card_id.clone(), "SQLite Meta Campaign");
    campaign.id = campaign_id.clone();
    sqlite_runtime::save_campaign(&campaign).expect("save campaign");
    sqlite_runtime::save_card_payload(
        &card_id,
        "SQLite Meta Fixture",
        Some("meta-source-1"),
        Some("2026-07-16T00:00:00Z"),
        &serde_json::to_value(&stored_card).expect("serialize stored card"),
    )
    .expect("save card payload");

    // Deliberately orphan the instance from the one valid definition. The
    // health result must therefore prove the live SQLite rows were inspected.
    let instance = CharacterInstance {
        id: Id::from_str("meta-instance-1"),
        campaign_id: campaign_id.clone(),
        definition_id: Some(Id::from_str("missing-definition")),
        name: "Orphan".into(),
        persona_override: None,
        behavior_override: None,
        variables: vec![],
        is_temporary: false,
    };
    sqlite_runtime::save_instance(&instance).expect("save orphan instance");
    let mut private = CharacterKnowledgeEntry::backstory(
        campaign_id.clone(),
        instance.id.clone(),
        "synthetic owner-only fixture fact",
    );
    private.set_propagation(PropagationPolicy::Private);
    sqlite_runtime::save_knowledge(&private).expect("save private knowledge");
    let mut task = StoryTask::user_planned(
        campaign_id.clone(),
        "Audit task",
        "Synthetic task for SQLite Meta inspection",
        vec![TaskTrigger::Manual],
        0,
    );
    task.id = Id::from_str("meta-task-1");
    sqlite_runtime::save_task(&task).expect("save task");
    assert_eq!(
        sqlite_runtime::list_knowledge(&campaign_id).unwrap().len(),
        1
    );
    assert_eq!(sqlite_runtime::list_tasks(&campaign_id).unwrap().len(), 1);

    // A legacy JSON decoy is removed before the query. A fallback cannot pass.
    let legacy_campaigns = temp.path().join("campaigns.json");
    fs::write(&legacy_campaigns, b"[]").expect("write JSON decoy");
    fs::remove_file(&legacy_campaigns).expect("remove JSON decoy");

    let issues = meta_backend::sqlite_campaign_health_issues(&campaign_id)
        .expect("health must read SQLite authority");
    assert!(
        issues
            .iter()
            .any(|issue| issue.category == "orphan_instance"),
        "SQLite orphan instance must be reported"
    );
    assert!(!legacy_campaigns.exists(), "health must not recreate JSON");

    // Until a typed-patch SQLite transaction exists, accept/preview must say
    // unsupported instead of touching a disabled or stale JSON store.
    assert!(meta_backend::ensure_typed_patch_backend_supported(false).is_ok());
    let unsupported = meta_backend::ensure_typed_patch_backend_supported(true)
        .expect_err("SQLite typed patch must fail closed for now");
    assert!(unsupported.contains("SQLite"));

    for capability in [
        "legacy Meta patch accept",
        "campaign repair proposals",
        "MVU translation persistence",
    ] {
        assert!(
            meta_backend::ensure_json_meta_backend_supported(true, capability).is_err(),
            "{capability} must not read or write the disabled JSON authority"
        );
        assert!(
            meta_backend::ensure_json_meta_backend_supported(false, capability).is_ok(),
            "{capability} remains available on the JSON backend"
        );
    }

    // A malformed authoritative card payload is a storage-integrity error,
    // never an empty-definition fallback that could hide broken references.
    sqlite_runtime::save_card_payload(
        &card_id,
        "SQLite Meta Fixture",
        Some("meta-source-1"),
        Some("2026-07-16T00:00:00Z"),
        &serde_json::json!({"card": {"id": "broken"}}),
    )
    .expect("replace card payload");
    assert!(meta_backend::sqlite_campaign_health_issues(&campaign_id).is_err());
}

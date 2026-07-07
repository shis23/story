//! Bronze deterministic harness coverage that does not require a real LLM.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use storyforge_domain::Id;
use storyforge_domain::campaign::{Campaign, CharacterInstance};
use storyforge_domain::character_knowledge::{
    CharacterKnowledgeUpdate, KnowledgeSource, PropagationPolicy,
};
use storyforge_tauri_app::campaign_store::CampaignStore;
use storyforge_tauri_app::normalize_knowledge_update_for_postprocess;

struct TempDataDir {
    path: PathBuf,
}

impl TempDataDir {
    fn new(prefix: &str) -> Self {
        let path = std::env::temp_dir().join(format!("{prefix}_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&path).expect("create temp data dir");
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDataDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

#[test]
fn bronze_same_name_instances_are_campaign_scoped_and_postprocess_persists() {
    let temp = TempDataDir::new("sf_bronze_same_name_campaign_scope");
    let store = CampaignStore::new(temp.path());

    let card_id = Id::from_str("card-bronze");
    let mut campaign_a = Campaign::new(card_id.clone(), "bronze-a");
    campaign_a.id = Id::from_str("campaign-a");
    let mut campaign_b = Campaign::new(card_id, "bronze-b");
    campaign_b.id = Id::from_str("campaign-b");
    store.save_campaign(campaign_a.clone()).unwrap();
    store.save_campaign(campaign_b.clone()).unwrap();

    let instance_a = named_instance(&campaign_a.id, "instance-a", "Echo");
    let instance_b = named_instance(&campaign_b.id, "instance-b", "Echo");
    store.add_instance(instance_a.clone()).unwrap();
    store.add_instance(instance_b.clone()).unwrap();

    let present = HashSet::from([String::from("Echo")]);
    let name_collisions = HashSet::new();

    let entries_a = normalize_knowledge_update_for_postprocess(
        &store,
        &campaign_a.id,
        &witnessed_update("Echo", "Echo saw the blue door"),
        7,
        &present,
        &name_collisions,
    );
    let entries_b = normalize_knowledge_update_for_postprocess(
        &store,
        &campaign_b.id,
        &witnessed_update("Echo", "Echo saw the red door"),
        8,
        &present,
        &name_collisions,
    );

    assert_eq!(entries_a.len(), 1, "campaign A should resolve its Echo");
    assert_eq!(entries_b.len(), 1, "campaign B should resolve its Echo");
    assert_eq!(entries_a[0].campaign_id, campaign_a.id);
    assert_eq!(entries_a[0].character_id, instance_a.id);
    assert_eq!(entries_b[0].campaign_id, campaign_b.id);
    assert_eq!(entries_b[0].character_id, instance_b.id);

    store.add_knowledge(entries_a).unwrap();
    store.add_knowledge(entries_b).unwrap();

    let reloaded = CampaignStore::new(temp.path());
    let knowledge_a = reloaded.list_knowledge(&campaign_a.id);
    let knowledge_b = reloaded.list_knowledge(&campaign_b.id);

    assert_eq!(knowledge_a.len(), 1);
    assert_eq!(knowledge_a[0].character_id, instance_a.id);
    assert_eq!(knowledge_a[0].knowledge_text, "Echo saw the blue door");
    assert_eq!(knowledge_a[0].turn_number, 7);

    assert_eq!(knowledge_b.len(), 1);
    assert_eq!(knowledge_b[0].character_id, instance_b.id);
    assert_eq!(knowledge_b[0].knowledge_text, "Echo saw the red door");
    assert_eq!(knowledge_b[0].turn_number, 8);
}

fn named_instance(campaign_id: &Id, instance_id: &str, name: &str) -> CharacterInstance {
    CharacterInstance {
        id: Id::from_str(instance_id),
        campaign_id: campaign_id.clone(),
        definition_id: None,
        name: name.to_string(),
        persona_override: None,
        behavior_override: None,
        variables: vec![],
        is_temporary: false,
    }
}

fn witnessed_update(character_name: &str, knowledge_text: &str) -> CharacterKnowledgeUpdate {
    CharacterKnowledgeUpdate {
        character_id: Id::from_str(character_name),
        knowledge_text: knowledge_text.to_string(),
        source: KnowledgeSource::Witnessed,
        source_character_id: None,
        pinned: false,
        broadcast: None,
        propagation: PropagationPolicy::Open,
    }
}

//! Campaign runtime snapshot (Phase 2 domain DTO)
//!
//! `CampaignRuntimeContext` is a pure domain snapshot that bundles all data
//! the writing pipeline needs from an active Campaign. It is assembled by
//! the Tauri layer from `CampaignStore` and passed down to `app-pipeline`
//! and `app-agent` as a read-only `Arc<CampaignRuntimeContext>`.
//!
//! This crate must NOT depend on `tauri-app`. All fields are cloneable
//! domain types only.

use std::collections::HashMap;

use crate::Id;
use crate::campaign::{Campaign, CharacterInstance};
use crate::character::CharacterDefinition;
use crate::character_knowledge::CharacterKnowledgeEntry;

/// Read-only snapshot of a Campaign's runtime state.
///
/// Assembled by the Tauri layer from `CampaignStore`, consumed by
/// `app-pipeline` (`WritingContext`) and `app-agent` (`ToolContext`).
/// Contains no store references, locks, or Tauri state.
#[derive(Debug, Clone)]
pub struct CampaignRuntimeContext {
    pub campaign: Campaign,
    pub instances: Vec<CharacterInstance>,
    pub definitions_by_id: HashMap<Id, CharacterDefinition>,
    pub knowledge: Vec<CharacterKnowledgeEntry>,
    pub turn: u32,
}

impl CampaignRuntimeContext {
    /// Look up the `CharacterDefinition` for an instance via `definition_id`.
    pub fn definition_for_instance(
        &self,
        instance: &CharacterInstance,
    ) -> Option<&CharacterDefinition> {
        instance
            .definition_id
            .as_ref()
            .and_then(|did| self.definitions_by_id.get(did))
    }

    /// Find an instance by ID or name.
    ///
    /// Priority: exact `instance.id` match first, then first `instance.name` match.
    pub fn find_instance_by_id_or_name(&self, value: &str) -> Option<&CharacterInstance> {
        // ID match (priority)
        if let Some(inst) = self.instances.iter().find(|i| i.id.as_str() == value) {
            return Some(inst);
        }
        // Name match (fallback)
        self.instances.iter().find(|i| i.name == value)
    }

    /// Resolved persona for an instance (override → definition → None).
    ///
    /// Delegates to `CharacterInstance::resolved_persona` with the
    /// instance's definition looked up from this context.
    pub fn resolved_persona_for<'a>(&'a self, instance: &'a CharacterInstance) -> Option<&'a str> {
        let def = self.definition_for_instance(instance);
        instance.resolved_persona(def)
    }

    /// Resolved behavior for an instance (override → definition → None).
    pub fn resolved_behavior_for<'a>(
        &'a self,
        instance: &'a CharacterInstance,
    ) -> Option<&'a str> {
        let def = self.definition_for_instance(instance);
        instance.resolved_behavior(def)
    }

    /// All knowledge entries belonging to a specific instance.
    pub fn knowledge_for_instance(
        &self,
        instance: &CharacterInstance,
    ) -> Vec<&CharacterKnowledgeEntry> {
        self.knowledge
            .iter()
            .filter(|k| k.character_id == instance.id)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::campaign::CharacterInstance;
    use crate::character::{CharacterDefinition, RoleType};
    use crate::character_knowledge::CharacterKnowledgeEntry;
    use crate::variables::default_character_variables;

    fn make_campaign() -> Campaign {
        Campaign::new(Id::from_str("card-1"), "test-campaign")
    }

    fn make_definition(id: &str, name: &str, persona: &str, behavior: &str) -> CharacterDefinition {
        CharacterDefinition {
            id: Id::from_str(id),
            card_id: Id::from_str("card-1"),
            name: name.into(),
            persona_prompt: persona.into(),
            behavior_rules: behavior.into(),
            base_backstory: vec![format!("{name} backstory")],
            group: None,
            role_type: RoleType::Protagonist,
            variable_schema: default_character_variables(),
        }
    }

    fn make_instance(
        id: &str,
        campaign_id: &Id,
        definition_id: Option<&str>,
        name: &str,
    ) -> CharacterInstance {
        CharacterInstance {
            id: Id::from_str(id),
            campaign_id: campaign_id.clone(),
            definition_id: definition_id.map(Id::from_str),
            name: name.into(),
            persona_override: None,
            behavior_override: None,
            variables: vec![],
            is_temporary: false,
        }
    }

    fn make_context() -> CampaignRuntimeContext {
        let campaign = make_campaign();
        let def_lin = make_definition("def-lin", "Lin", "calm surgeon", "save first");
        let def_chen = make_definition("def-chen", "Chen", "strict cop", "follow rules");

        let inst_lin = make_instance("inst-lin", &campaign.id, Some("def-lin"), "Lin");
        let inst_chen = make_instance("inst-chen", &campaign.id, Some("def-chen"), "Chen");
        let inst_ghost = make_instance("inst-ghost", &campaign.id, None, "Ghost");

        let mut definitions_by_id = HashMap::new();
        definitions_by_id.insert(def_lin.id.clone(), def_lin);
        definitions_by_id.insert(def_chen.id.clone(), def_chen);

        CampaignRuntimeContext {
            campaign,
            instances: vec![inst_lin, inst_chen, inst_ghost],
            definitions_by_id,
            knowledge: vec![],
            turn: 1,
        }
    }

    // --- find_instance_by_id_or_name ---

    #[test]
    fn find_by_id() {
        let ctx = make_context();
        let inst = ctx.find_instance_by_id_or_name("inst-lin").unwrap();
        assert_eq!(inst.name, "Lin");
    }

    #[test]
    fn find_by_name() {
        let ctx = make_context();
        let inst = ctx.find_instance_by_id_or_name("Chen").unwrap();
        assert_eq!(inst.id.as_str(), "inst-chen");
    }

    #[test]
    fn id_takes_priority_over_name() {
        // Create a context where an instance has name == another instance's id
        let campaign = make_campaign();
        let inst_a = make_instance("inst-a", &campaign.id, None, "inst-b"); // name = "inst-b"
        let inst_b = make_instance("inst-b", &campaign.id, None, "Other"); // id = "inst-b"

        let ctx = CampaignRuntimeContext {
            campaign,
            instances: vec![inst_a, inst_b],
            definitions_by_id: HashMap::new(),
            knowledge: vec![],
            turn: 1,
        };

        // "inst-b" matches both inst_a (by name) and inst_b (by id).
        // ID match must win.
        let found = ctx.find_instance_by_id_or_name("inst-b").unwrap();
        assert_eq!(found.id.as_str(), "inst-b");
        assert_eq!(found.name, "Other");
    }

    #[test]
    fn find_returns_none_for_unknown() {
        let ctx = make_context();
        assert!(ctx.find_instance_by_id_or_name("nonexistent").is_none());
    }

    // --- definition_for_instance ---

    #[test]
    fn definition_for_instance_returns_definition() {
        let ctx = make_context();
        let inst = ctx.find_instance_by_id_or_name("inst-lin").unwrap();
        let def = ctx.definition_for_instance(inst).unwrap();
        assert_eq!(def.name, "Lin");
        assert_eq!(def.persona_prompt, "calm surgeon");
    }

    #[test]
    fn definition_for_instance_none_when_no_definition_id() {
        let ctx = make_context();
        let inst = ctx.find_instance_by_id_or_name("Ghost").unwrap();
        assert!(inst.definition_id.is_none());
        assert!(ctx.definition_for_instance(inst).is_none());
    }

    #[test]
    fn definition_for_instance_none_when_id_not_in_map() {
        let campaign = make_campaign();
        let inst = make_instance("inst-x", &campaign.id, Some("def-missing"), "X");
        let ctx = CampaignRuntimeContext {
            campaign,
            instances: vec![inst],
            definitions_by_id: HashMap::new(),
            knowledge: vec![],
            turn: 1,
        };
        let inst = ctx.find_instance_by_id_or_name("inst-x").unwrap();
        assert!(ctx.definition_for_instance(inst).is_none());
    }

    // --- resolved_persona_for / resolved_behavior_for ---

    #[test]
    fn resolved_persona_for_uses_definition_fallback() {
        let ctx = make_context();
        let inst = ctx.find_instance_by_id_or_name("inst-lin").unwrap();
        // No persona_override → falls back to definition.persona_prompt
        assert_eq!(ctx.resolved_persona_for(inst), Some("calm surgeon"));
    }

    #[test]
    fn resolved_behavior_for_uses_definition_fallback() {
        let ctx = make_context();
        let inst = ctx.find_instance_by_id_or_name("inst-chen").unwrap();
        assert_eq!(ctx.resolved_behavior_for(inst), Some("follow rules"));
    }

    #[test]
    fn resolved_persona_for_override_takes_priority() {
        let ctx = make_context();
        let mut inst = ctx.find_instance_by_id_or_name("inst-lin").unwrap().clone();
        inst.persona_override = Some("angry variant".into());

        // Even though definition exists, override wins
        assert_eq!(ctx.resolved_persona_for(&inst), Some("angry variant"));
    }

    #[test]
    fn resolved_persona_for_none_when_no_definition_no_override() {
        let ctx = make_context();
        let inst = ctx.find_instance_by_id_or_name("Ghost").unwrap();
        // Ghost has no definition_id and no override
        assert_eq!(ctx.resolved_persona_for(inst), None);
    }

    #[test]
    fn resolved_behavior_for_none_when_no_definition_no_override() {
        let ctx = make_context();
        let inst = ctx.find_instance_by_id_or_name("Ghost").unwrap();
        assert_eq!(ctx.resolved_behavior_for(inst), None);
    }

    #[test]
    fn resolved_persona_for_override_empty_string_still_wins() {
        let campaign = make_campaign();
        let def = make_definition("def-1", "A", "persona-text", "behavior-text");
        let mut inst = make_instance("inst-1", &campaign.id, Some("def-1"), "A");
        inst.persona_override = Some("".into());

        let mut definitions_by_id = HashMap::new();
        definitions_by_id.insert(def.id.clone(), def);

        let ctx = CampaignRuntimeContext {
            campaign,
            instances: vec![inst],
            definitions_by_id,
            knowledge: vec![],
            turn: 1,
        };
        let inst = ctx.find_instance_by_id_or_name("inst-1").unwrap();
        assert_eq!(ctx.resolved_persona_for(inst), Some(""));
    }

    // --- knowledge_for_instance ---

    #[test]
    fn knowledge_for_instance_filters_by_id() {
        let campaign = make_campaign();
        let inst_lin = make_instance("inst-lin", &campaign.id, None, "Lin");
        let inst_chen = make_instance("inst-chen", &campaign.id, None, "Chen");

        let knowledge = vec![
            CharacterKnowledgeEntry::witnessed(
                campaign.id.clone(),
                Id::from_str("inst-lin"),
                "Lin saw the explosion",
                1,
            ),
            CharacterKnowledgeEntry::witnessed(
                campaign.id.clone(),
                Id::from_str("inst-chen"),
                "Chen was at the station",
                1,
            ),
            CharacterKnowledgeEntry::told_by(
                campaign.id.clone(),
                Id::from_str("inst-lin"),
                "Chen told Lin about the body",
                Id::from_str("inst-chen"),
                2,
            ),
        ];

        let ctx = CampaignRuntimeContext {
            campaign,
            instances: vec![inst_lin, inst_chen],
            definitions_by_id: HashMap::new(),
            knowledge,
            turn: 3,
        };

        let lin_knowledge = ctx.knowledge_for_instance(ctx.find_instance_by_id_or_name("inst-lin").unwrap());
        assert_eq!(lin_knowledge.len(), 2);
        assert!(lin_knowledge.iter().all(|k| k.character_id == Id::from_str("inst-lin")));

        let chen_knowledge = ctx.knowledge_for_instance(ctx.find_instance_by_id_or_name("inst-chen").unwrap());
        assert_eq!(chen_knowledge.len(), 1);
        assert_eq!(chen_knowledge[0].knowledge_text, "Chen was at the station");
    }

    #[test]
    fn knowledge_for_instance_empty_when_no_entries() {
        let ctx = make_context();
        let inst = ctx.find_instance_by_id_or_name("inst-lin").unwrap();
        assert!(ctx.knowledge_for_instance(inst).is_empty());
    }
}

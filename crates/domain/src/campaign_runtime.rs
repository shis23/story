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
use crate::story_task::StoryTask;

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
    /// Campaign 的任务列表（Phase 3 阶段 4：供 Meta inspect_tasks / propose_campaign_patch 读取）
    pub tasks: Vec<StoryTask>,
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
    pub fn resolved_behavior_for<'a>(&'a self, instance: &'a CharacterInstance) -> Option<&'a str> {
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

    /// Create a new context with temporary instances for unmatched character IDs.
    ///
    /// Phase 6: When the Director produces tasks for characters not in the Campaign,
    /// this creates `CharacterInstance::temporary_with_overrides` for each unmatched ID
    /// and adds them to the context. Existing instances are preserved.
    ///
    /// `character_specs` is a slice of `(character_id, persona_override, behavior_override)`.
    /// The persona/behavior overrides are applied to newly created temporary instances
    /// (e.g., from Director's `context_package.character_brief`).
    ///
    /// Returns a new `CampaignRuntimeContext` with temporaries added, and a list
    /// of the newly created temporary instances (for the caller to persist if needed).
    pub fn with_temporaries_for(
        &self,
        character_specs: &[(String, Option<String>, Option<String>)],
    ) -> (Self, Vec<CharacterInstance>) {
        let mut new_instances = self.instances.clone();
        let mut new_temps = Vec::new();
        let mut seen: std::collections::HashSet<String> = self
            .instances
            .iter()
            // ID 与名称统一小写去重：混用大小写的实例 ID（"Inst-1" vs "inst-1"）
            // 曾绕过去重生成重复临时实例（2026-09-01 全量审查修复）
            .flat_map(|inst| [inst.id.as_str().to_lowercase(), inst.name.to_lowercase()])
            .collect();

        for (cid, persona, behavior) in character_specs {
            let cid = cid.trim();
            if cid.is_empty() {
                continue;
            }

            // Skip if already matched (IDs are case-insensitive UUIDs; names are lowercased)
            if !seen.insert(cid.to_lowercase()) {
                continue;
            }
            // Create temporary instance with optional overrides
            let temp = CharacterInstance::temporary_with_overrides(
                self.campaign.id.clone(),
                cid,
                persona.clone(),
                behavior.clone(),
            );
            new_temps.push(temp.clone());
            new_instances.push(temp);
        }

        let new_ctx = Self {
            campaign: self.campaign.clone(),
            instances: new_instances,
            definitions_by_id: self.definitions_by_id.clone(),
            knowledge: self.knowledge.clone(),
            tasks: self.tasks.clone(),
            turn: self.turn,
        };
        (new_ctx, new_temps)
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
            tasks: vec![],
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
            tasks: vec![],
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
            tasks: vec![],
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
            tasks: vec![],
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
            tasks: vec![],
            turn: 3,
        };

        let lin_knowledge =
            ctx.knowledge_for_instance(ctx.find_instance_by_id_or_name("inst-lin").unwrap());
        assert_eq!(lin_knowledge.len(), 2);
        assert!(
            lin_knowledge
                .iter()
                .all(|k| k.character_id == Id::from_str("inst-lin"))
        );

        let chen_knowledge =
            ctx.knowledge_for_instance(ctx.find_instance_by_id_or_name("inst-chen").unwrap());
        assert_eq!(chen_knowledge.len(), 1);
        assert_eq!(chen_knowledge[0].knowledge_text, "Chen was at the station");
    }

    #[test]
    fn knowledge_for_instance_empty_when_no_entries() {
        let ctx = make_context();
        let inst = ctx.find_instance_by_id_or_name("inst-lin").unwrap();
        assert!(ctx.knowledge_for_instance(inst).is_empty());
    }

    // --- Phase 6: with_temporaries_for ---

    #[test]
    fn with_temporaries_creates_for_unmatched() {
        let ctx = make_context();
        let specs: Vec<(String, Option<String>, Option<String>)> = vec![
            ("inst-lin".into(), None, None),
            ("NewCharacter".into(), None, None),
            ("inst-chen".into(), None, None),
        ];
        let (new_ctx, temps) = ctx.with_temporaries_for(&specs);

        // inst-lin and inst-chen already exist, so only NewCharacter gets a temporary
        assert_eq!(temps.len(), 1);
        assert_eq!(new_ctx.instances.len(), 4); // 3 original + 1 temporary

        // The temporary instance should be findable by name
        let temp = new_ctx.find_instance_by_id_or_name("NewCharacter").unwrap();
        assert!(temp.is_temporary);
        assert!(temp.definition_id.is_none());
        assert_eq!(temp.name, "NewCharacter");
    }

    #[test]
    fn with_temporaries_skips_existing() {
        let ctx = make_context();
        let specs: Vec<(String, Option<String>, Option<String>)> =
            vec![("inst-lin".into(), None, None)];
        let (new_ctx, temps) = ctx.with_temporaries_for(&specs);

        assert!(temps.is_empty());
        assert_eq!(new_ctx.instances.len(), 3); // unchanged
    }

    #[test]
    fn with_temporaries_dedups_duplicate_unmatched_specs() {
        let ctx = make_context();
        let specs: Vec<(String, Option<String>, Option<String>)> = vec![
            ("Wanderer".into(), Some("first brief".into()), None),
            ("Wanderer".into(), Some("second brief".into()), None),
        ];
        let (new_ctx, temps) = ctx.with_temporaries_for(&specs);

        assert_eq!(temps.len(), 1);
        assert_eq!(temps[0].name, "Wanderer");
        assert_eq!(temps[0].persona_override, Some("first brief".into()));
        assert_eq!(
            new_ctx
                .instances
                .iter()
                .filter(|inst| inst.name == "Wanderer")
                .count(),
            1
        );
    }

    #[test]
    fn with_temporaries_skips_blank_unmatched_specs() {
        let ctx = make_context();
        let specs: Vec<(String, Option<String>, Option<String>)> = vec![
            ("".into(), None, None),
            ("   ".into(), Some("blank brief".into()), None),
            ("NamelessWitness".into(), None, None),
        ];
        let (new_ctx, temps) = ctx.with_temporaries_for(&specs);

        assert_eq!(temps.len(), 1);
        assert_eq!(temps[0].name, "NamelessWitness");
        assert!(
            new_ctx
                .instances
                .iter()
                .all(|inst| inst.name != "Unknown Character")
        );
    }

    #[test]
    fn with_temporaries_preserves_existing_data() {
        let ctx = make_context();
        let specs: Vec<(String, Option<String>, Option<String>)> =
            vec![("Ghost".into(), None, None)];
        let (new_ctx, _temps) = ctx.with_temporaries_for(&specs);

        // Original instances preserved
        assert!(new_ctx.find_instance_by_id_or_name("inst-lin").is_some());
        assert!(new_ctx.find_instance_by_id_or_name("inst-chen").is_some());

        // Definitions preserved
        assert_eq!(new_ctx.definitions_by_id.len(), 2);

        // Campaign preserved
        assert_eq!(new_ctx.campaign.id, ctx.campaign.id);
    }

    #[test]
    fn temporary_instance_gets_default_variables() {
        let ctx = make_context();
        let specs: Vec<(String, Option<String>, Option<String>)> =
            vec![("AdHoc".into(), None, None)];
        let (new_ctx, _temps) = ctx.with_temporaries_for(&specs);

        let temp = new_ctx.find_instance_by_id_or_name("AdHoc").unwrap();
        // CharacterInstance::temporary initializes default variables (hp etc.)
        assert!(
            !temp.variables.is_empty(),
            "temporary should have default variables"
        );
    }

    #[test]
    fn with_temporaries_passes_persona_override() {
        let ctx = make_context();
        let specs: Vec<(String, Option<String>, Option<String>)> =
            vec![("NewChar".into(), Some("mysterious stranger".into()), None)];
        let (new_ctx, temps) = ctx.with_temporaries_for(&specs);

        assert_eq!(temps.len(), 1);
        let temp = &temps[0];
        assert_eq!(temp.name, "NewChar");
        assert_eq!(temp.persona_override, Some("mysterious stranger".into()));
        assert!(temp.behavior_override.is_none());

        // Also findable in the new context
        let found = new_ctx.find_instance_by_id_or_name("NewChar").unwrap();
        assert_eq!(found.id, temp.id);
        assert_eq!(found.resolved_persona(None), Some("mysterious stranger"));
    }

    #[test]
    fn with_temporaries_passes_behavior_override() {
        let ctx = make_context();
        let specs: Vec<(String, Option<String>, Option<String>)> =
            vec![("Guard".into(), None, Some("block the passage".into()))];
        let (new_ctx, temps) = ctx.with_temporaries_for(&specs);

        assert_eq!(temps.len(), 1);
        assert_eq!(temps[0].behavior_override, Some("block the passage".into()));

        let found = new_ctx.find_instance_by_id_or_name("Guard").unwrap();
        assert_eq!(found.resolved_behavior(None), Some("block the passage"));
    }

    #[test]
    fn with_temporaries_passes_both_overrides() {
        let ctx = make_context();
        let specs: Vec<(String, Option<String>, Option<String>)> = vec![(
            "Shopkeeper".into(),
            Some("friendly merchant".into()),
            Some("offer fair prices".into()),
        )];
        let (_new_ctx, temps) = ctx.with_temporaries_for(&specs);

        assert_eq!(temps.len(), 1);
        assert_eq!(temps[0].persona_override, Some("friendly merchant".into()));
        assert_eq!(temps[0].behavior_override, Some("offer fair prices".into()));
    }
}

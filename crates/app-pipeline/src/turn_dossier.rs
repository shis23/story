use std::collections::HashSet;

use storyforge_domain::campaign_runtime::CampaignRuntimeContext;
use storyforge_domain::character::Character;
use storyforge_domain::character_knowledge::PropagationPolicy;
use storyforge_domain::story_task::StoryTask;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum KnowledgeVisibility {
    Open,
    Private,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DossierKnowledge {
    pub id: String,
    pub text: String,
    pub visibility: KnowledgeVisibility,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RosterEntry {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ActorDossier {
    pub id: String,
    pub name: String,
    pub persona: String,
    pub behavior: String,
    pub agenda: Option<String>,
    pub variables: Vec<(String, String)>,
    pub knowledge: Vec<DossierKnowledge>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CognitiveBoundary {
    pub fact_id: String,
    pub fact_text: String,
    pub knower_id: String,
    pub excluded_actor_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CompiledTurnDossier {
    pub intent: String,
    pub roster: Vec<RosterEntry>,
    pub actors: Vec<ActorDossier>,
    pub cognitive_boundaries: Vec<CognitiveBoundary>,
    pub pending_tasks: String,
}

impl CompiledTurnDossier {
    pub(crate) fn render_for_writer(&self) -> String {
        let mut sections = vec![format!("## 用户意图\n{}", self.intent.trim())];
        let roster = self
            .roster
            .iter()
            .map(|actor| format!("- {}（{}）", actor.name, actor.id))
            .collect::<Vec<_>>()
            .join("\n");
        sections.push(format!("## 全名册（每人仅一行）\n{roster}"));

        let mut actor_blocks = Vec::new();
        for actor in &self.actors {
            let mut lines = vec![format!("### {}（{}）", actor.name, actor.id)];
            if !actor.persona.trim().is_empty() {
                lines.push(format!("人设：{}", actor.persona.trim()));
            }
            if !actor.behavior.trim().is_empty() {
                lines.push(format!("行为准则：{}", actor.behavior.trim()));
            }
            if let Some(agenda) = &actor.agenda {
                lines.push(format!("当前议程：{agenda}"));
            }
            if !actor.variables.is_empty() {
                lines.push(format!(
                    "关键变量：{}",
                    actor
                        .variables
                        .iter()
                        .map(|(key, value)| format!("{key}={value}"))
                        .collect::<Vec<_>>()
                        .join("；")
                ));
            }
            for fact in &actor.knowledge {
                let label = match fact.visibility {
                    KnowledgeVisibility::Open => format!("[{}知道]", actor.name),
                    KnowledgeVisibility::Private => {
                        format!("[仅{}知道·秘密]", actor.name)
                    }
                };
                lines.push(format!("{label} {}", fact.text));
            }
            actor_blocks.push(lines.join("\n"));
        }
        if !actor_blocks.is_empty() {
            sections.push(format!(
                "## 预期在场角色档案\n{}",
                actor_blocks.join("\n\n")
            ));
        }

        let mut boundary_lines = Vec::new();
        for boundary in &self.cognitive_boundaries {
            for excluded_id in &boundary.excluded_actor_ids {
                let excluded_name = self
                    .roster
                    .iter()
                    .find(|actor| &actor.id == excluded_id)
                    .map(|actor| actor.name.as_str())
                    .unwrap_or(excluded_id);
                boundary_lines.push(format!(
                    "- {excluded_name}不知道：{}（仅 {} 持有；事实 id={}）",
                    boundary.fact_text, boundary.knower_id, boundary.fact_id
                ));
            }
        }
        if !boundary_lines.is_empty() {
            sections.push(format!(
                "## 认知边界（硬约束）\n{}\n不得让无知者在被公开告知前说出、确认或据此行动。",
                boundary_lines.join("\n")
            ));
        }
        if !self.pending_tasks.trim().is_empty() {
            sections.push(format!("## 未了任务与伏笔\n{}", self.pending_tasks.trim()));
        }
        sections.join("\n\n")
    }
}

fn json_value_text(value: &serde_json::Value) -> String {
    value
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| value.to_string())
}

fn normalized_fact(text: &str) -> String {
    text.chars()
        .filter(|character| !character.is_whitespace() && !character.is_ascii_punctuation())
        .flat_map(char::to_lowercase)
        .collect()
}

pub(crate) fn compile_turn_dossier(
    intent: &str,
    runtime: &CampaignRuntimeContext,
    pending_tasks: &[StoryTask],
    max_full_actors: usize,
) -> CompiledTurnDossier {
    let roster = runtime
        .instances
        .iter()
        .map(|instance| RosterEntry {
            id: instance.id.as_str().to_string(),
            name: instance.name.clone(),
        })
        .collect::<Vec<_>>();

    let mentioned_ids = runtime
        .instances
        .iter()
        .filter(|instance| intent.contains(&instance.name) || intent.contains(instance.id.as_str()))
        .map(|instance| instance.id.as_str().to_string())
        .collect::<HashSet<_>>();
    let task_related_ids = pending_tasks
        .iter()
        .filter(|task| task.status.is_injectable())
        .flat_map(|task| task.related_characters.iter())
        .map(|id| id.as_str().to_string())
        .collect::<HashSet<_>>();

    let mut ranked = runtime.instances.iter().enumerate().collect::<Vec<_>>();
    ranked.sort_by_key(|(original_index, instance)| {
        let id = instance.id.as_str();
        let priority = if mentioned_ids.contains(id) {
            0
        } else if task_related_ids.contains(id) {
            1
        } else {
            2
        };
        (priority, *original_index)
    });
    let selected = ranked
        .into_iter()
        .take(max_full_actors)
        .map(|(_, instance)| instance)
        .collect::<Vec<_>>();
    let selected_ids = selected
        .iter()
        .map(|instance| instance.id.as_str().to_string())
        .collect::<HashSet<_>>();

    let actors = selected
        .iter()
        .map(|instance| {
            let agenda = instance
                .variables
                .iter()
                .find(|value| {
                    matches!(
                        value.key.as_str(),
                        "agenda" | "current_desire" | "goal" | "ongoing_action"
                    )
                })
                .map(|value| json_value_text(&value.value))
                .filter(|value| !value.trim().is_empty());
            let variables = instance
                .variables
                .iter()
                .filter(|value| {
                    !matches!(
                        value.key.as_str(),
                        "agenda" | "current_desire" | "goal" | "ongoing_action"
                    )
                })
                .map(|value| (value.key.clone(), json_value_text(&value.value)))
                .collect();
            let knowledge = runtime
                .knowledge_for_instance(instance)
                .into_iter()
                .map(|fact| DossierKnowledge {
                    id: fact.id.as_str().to_string(),
                    text: fact.knowledge_text.clone(),
                    visibility: if matches!(fact.propagation, PropagationPolicy::Private) {
                        KnowledgeVisibility::Private
                    } else {
                        KnowledgeVisibility::Open
                    },
                })
                .collect();
            ActorDossier {
                id: instance.id.as_str().to_string(),
                name: instance.name.clone(),
                persona: runtime
                    .resolved_persona_for(instance)
                    .unwrap_or_default()
                    .to_string(),
                behavior: runtime
                    .resolved_behavior_for(instance)
                    .unwrap_or_default()
                    .to_string(),
                agenda,
                variables,
                knowledge,
            }
        })
        .collect::<Vec<_>>();

    let cognitive_boundaries = runtime
        .knowledge
        .iter()
        .filter(|fact| {
            selected_ids.contains(fact.character_id.as_str())
                && matches!(fact.propagation, PropagationPolicy::Private)
        })
        .filter_map(|fact| {
            let normalized = normalized_fact(&fact.knowledge_text);
            let excluded_actor_ids = selected
                .iter()
                .filter(|instance| instance.id != fact.character_id)
                .filter(|instance| {
                    !runtime
                        .knowledge_for_instance(instance)
                        .iter()
                        .any(|known| normalized_fact(&known.knowledge_text) == normalized)
                })
                .map(|instance| instance.id.as_str().to_string())
                .collect::<Vec<_>>();
            (!excluded_actor_ids.is_empty()).then(|| CognitiveBoundary {
                fact_id: fact.id.as_str().to_string(),
                fact_text: fact.knowledge_text.clone(),
                knower_id: fact.character_id.as_str().to_string(),
                excluded_actor_ids,
            })
        })
        .collect();

    let pending_tasks =
        storyforge_domain::story_task::render_tasks_for_injection(pending_tasks, runtime.turn, "");
    CompiledTurnDossier {
        intent: intent.to_string(),
        roster,
        actors,
        cognitive_boundaries,
        pending_tasks,
    }
}

pub(crate) fn compile_legacy_turn_dossier(
    intent: &str,
    characters: &[std::sync::Arc<Character>],
    max_full_actors: usize,
) -> CompiledTurnDossier {
    let roster = characters
        .iter()
        .map(|character| RosterEntry {
            id: character.id.as_str().to_string(),
            name: character.name.clone(),
        })
        .collect::<Vec<_>>();
    let mut ranked = characters.iter().enumerate().collect::<Vec<_>>();
    ranked.sort_by_key(|(index, character)| {
        let mentioned = intent.contains(&character.name) || intent.contains(character.id.as_str());
        (usize::from(!mentioned), *index)
    });
    let actors = ranked
        .into_iter()
        .take(max_full_actors)
        .map(|(_, character)| ActorDossier {
            id: character.id.as_str().to_string(),
            name: character.name.clone(),
            persona: [
                character.description.trim(),
                character.personality.trim(),
                character.scenario.trim(),
            ]
            .into_iter()
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join("\n"),
            behavior: character.post_history_instructions.clone(),
            agenda: None,
            variables: vec![],
            knowledge: vec![],
        })
        .collect();
    CompiledTurnDossier {
        intent: intent.to_string(),
        roster,
        actors,
        cognitive_boundaries: vec![],
        pending_tasks: String::new(),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use storyforge_domain::Id;
    use storyforge_domain::campaign::{Campaign, CharacterInstance};
    use storyforge_domain::campaign_runtime::CampaignRuntimeContext;
    use storyforge_domain::character_knowledge::{
        CharacterKnowledgeEntry, KnowledgeSource, PropagationPolicy,
    };
    use storyforge_domain::variables::VariableValue;

    use super::*;

    fn runtime() -> CampaignRuntimeContext {
        let campaign = Campaign::new(Id::from_str("card"), "案卷测试");
        let mut a = CharacterInstance::temporary(campaign.id.clone(), "林如");
        a.id = Id::from_str("actor-a");
        a.persona_override = Some("冷静的调查员".into());
        a.behavior_override = Some("先观察再开口".into());
        a.variables = vec![
            VariableValue::new("agenda", serde_json::json!("找出内鬼"), 2),
            VariableValue::new("mood", serde_json::json!("警惕"), 2),
        ];
        let mut b = CharacterInstance::temporary(campaign.id.clone(), "陈默");
        b.id = Id::from_str("actor-b");
        let mut c = CharacterInstance::temporary(campaign.id.clone(), "周岚");
        c.id = Id::from_str("actor-c");
        let mut d = CharacterInstance::temporary(campaign.id.clone(), "路人");
        d.id = Id::from_str("actor-d");

        let private = CharacterKnowledgeEntry {
            id: Id::from_str("knowledge-secret"),
            campaign_id: campaign.id.clone(),
            character_id: a.id.clone(),
            knowledge_text: "钥匙藏在钟里".into(),
            source: KnowledgeSource::Witnessed,
            source_character_id: None,
            source_knowledge_id: None,
            turn_number: 2,
            event_id: None,
            pinned: false,
            propagation: PropagationPolicy::Private,
        };
        let open = CharacterKnowledgeEntry {
            id: Id::from_str("knowledge-open"),
            campaign_id: campaign.id.clone(),
            character_id: b.id.clone(),
            knowledge_text: "今晚会下雨".into(),
            source: KnowledgeSource::ToldByOther,
            source_character_id: Some(a.id.clone()),
            source_knowledge_id: None,
            turn_number: 2,
            event_id: None,
            pinned: false,
            propagation: PropagationPolicy::Open,
        };

        CampaignRuntimeContext {
            campaign,
            instances: vec![a, b, c, d],
            definitions_by_id: HashMap::new(),
            knowledge: vec![private, open],
            tasks: vec![],
            turn: 3,
        }
    }

    #[test]
    fn compiler_prioritizes_named_actors_and_caps_full_dossiers_at_three() {
        let dossier = compile_turn_dossier("陈默逼问林如，周岚在门口旁听", &runtime(), &[], 3);

        assert_eq!(dossier.actors.len(), 3);
        assert_eq!(
            dossier
                .actors
                .iter()
                .map(|actor| actor.name.as_str())
                .collect::<Vec<_>>(),
            vec!["林如", "陈默", "周岚"]
        );
        assert_eq!(dossier.roster.len(), 4, "全名册仍应保留一行/人");
    }

    #[test]
    fn private_knowledge_is_owner_labeled_and_creates_conservative_boundary() {
        let dossier = compile_turn_dossier("林如质问陈默", &runtime(), &[], 3);
        let owner = dossier
            .actors
            .iter()
            .find(|actor| actor.id == "actor-a")
            .unwrap();
        let other = dossier
            .actors
            .iter()
            .find(|actor| actor.id == "actor-b")
            .unwrap();

        assert!(owner.knowledge.iter().any(|fact| {
            fact.text == "钥匙藏在钟里" && fact.visibility == KnowledgeVisibility::Private
        }));
        assert!(
            other
                .knowledge
                .iter()
                .all(|fact| fact.text != "钥匙藏在钟里")
        );
        assert!(dossier.cognitive_boundaries.iter().any(|boundary| {
            boundary.fact_id == "knowledge-secret"
                && boundary.knower_id == "actor-a"
                && boundary.excluded_actor_ids.contains(&"actor-b".to_string())
        }));
    }

    #[test]
    fn missing_open_fact_never_becomes_an_ignorance_boundary() {
        let dossier = compile_turn_dossier("林如质问陈默", &runtime(), &[], 3);

        assert!(
            dossier
                .cognitive_boundaries
                .iter()
                .all(|boundary| boundary.fact_id != "knowledge-open")
        );
    }

    #[test]
    fn rendered_dossier_carries_ids_agenda_variables_and_ownership_labels() {
        let dossier = compile_turn_dossier("林如质问陈默", &runtime(), &[], 3);
        let rendered = dossier.render_for_writer();

        assert!(rendered.contains("林如（actor-a）"));
        assert!(rendered.contains("当前议程：找出内鬼"));
        assert!(rendered.contains("mood=警惕"));
        assert!(rendered.contains("[仅林如知道·秘密] 钥匙藏在钟里"));
        assert!(rendered.contains("陈默不知道：钥匙藏在钟里"));
        assert!(rendered.contains("路人（actor-d）"), "全名册应完整");
    }
}

use super::super::*;

// ─── P2 后处理产出查询 / 任务管理命令（6 个）──────────────────────────────────

/// 角色知识条目 DTO（前端展示用）
#[derive(Debug, Clone, Serialize)]
pub struct KnowledgeEntryDto {
    pub id: String,
    pub campaign_id: String,
    pub character_id: String,
    pub character_name: Option<String>,
    pub knowledge_text: String,
    pub source: String,
    pub source_character_id: Option<String>,
    pub source_character_name: Option<String>,
    pub source_knowledge_id: Option<String>,
    pub relay_chain_text: Option<String>,
    pub provenance_text: String,
    pub turn_number: u32,
    pub pinned: bool,
    pub propagation: String,
}

pub fn knowledge_source_code(
    source: &storyforge_domain::character_knowledge::KnowledgeSource,
) -> &str {
    use storyforge_domain::character_knowledge::KnowledgeSource;
    match source {
        KnowledgeSource::Witnessed => "witnessed",
        KnowledgeSource::ToldByOther => "told_by_other",
        KnowledgeSource::Inferred => "inferred",
        KnowledgeSource::Backstory => "backstory",
    }
}

pub fn knowledge_provenance_text(
    entry: &storyforge_domain::character_knowledge::CharacterKnowledgeEntry,
    instance_names: &std::collections::HashMap<Id, String>,
) -> String {
    use storyforge_domain::character_knowledge::{KnowledgeSource, PropagationPolicy};
    let target = instance_names
        .get(&entry.character_id)
        .cloned()
        .unwrap_or_else(|| entry.character_id.to_string());
    let base = match entry.source {
        KnowledgeSource::Witnessed => format!("{target} 亲眼所见"),
        KnowledgeSource::ToldByOther => {
            let source = entry
                .source_character_id
                .as_ref()
                .and_then(|id| instance_names.get(id))
                .cloned()
                .or_else(|| entry.source_character_id.as_ref().map(|id| id.to_string()));
            match source {
                Some(source) => format!("{target} 被 {source} 告知"),
                None => format!("{target} 被告知"),
            }
        }
        KnowledgeSource::Inferred => format!("{target} 自行推断"),
        KnowledgeSource::Backstory => format!("{target} 的背景知识"),
    };

    match &entry.propagation {
        PropagationPolicy::Open => base,
        PropagationPolicy::Private => format!("{base}（秘密，禁止外传）"),
        PropagationPolicy::GroupRestricted(group) => format!("{base}（限制传播：仅{group}）"),
    }
}

pub fn knowledge_actor_label(
    entry: &storyforge_domain::character_knowledge::CharacterKnowledgeEntry,
    instance_names: &std::collections::HashMap<Id, String>,
) -> String {
    instance_names
        .get(&entry.character_id)
        .cloned()
        .unwrap_or_else(|| entry.character_id.to_string())
}

pub fn knowledge_relay_chain_text(
    entry: &storyforge_domain::character_knowledge::CharacterKnowledgeEntry,
    instance_names: &std::collections::HashMap<Id, String>,
    knowledge_by_id: &std::collections::HashMap<
        Id,
        &storyforge_domain::character_knowledge::CharacterKnowledgeEntry,
    >,
) -> Option<String> {
    let mut chain = vec![entry];
    let mut current = entry;
    let mut seen = std::collections::HashSet::from([entry.id.clone()]);

    while let Some(parent_id) = &current.source_knowledge_id {
        if !seen.insert(parent_id.clone()) {
            break;
        }
        let Some(parent) = knowledge_by_id.get(parent_id).copied() else {
            break;
        };
        chain.push(parent);
        current = parent;
        if chain.len() >= 8 {
            break;
        }
    }

    if chain.len() < 2 {
        return None;
    }

    chain.reverse();
    Some(
        chain
            .iter()
            .map(|entry| {
                format!(
                    "{}（轮 {}）",
                    knowledge_actor_label(entry, instance_names),
                    entry.turn_number
                )
            })
            .collect::<Vec<_>>()
            .join(" → "),
    )
}

pub fn propagation_policy_code(
    policy: &storyforge_domain::character_knowledge::PropagationPolicy,
) -> String {
    use storyforge_domain::character_knowledge::PropagationPolicy;
    match policy {
        PropagationPolicy::Open => "open".into(),
        PropagationPolicy::Private => "private".into(),
        PropagationPolicy::GroupRestricted(group) => format!("group:{group}"),
    }
}

pub fn knowledge_entry_to_dto(
    entry: &storyforge_domain::character_knowledge::CharacterKnowledgeEntry,
    instance_names: &std::collections::HashMap<Id, String>,
    knowledge_by_id: &std::collections::HashMap<
        Id,
        &storyforge_domain::character_knowledge::CharacterKnowledgeEntry,
    >,
) -> KnowledgeEntryDto {
    KnowledgeEntryDto {
        id: entry.id.to_string(),
        campaign_id: entry.campaign_id.to_string(),
        character_id: entry.character_id.to_string(),
        character_name: instance_names.get(&entry.character_id).cloned(),
        knowledge_text: entry.knowledge_text.clone(),
        source: knowledge_source_code(&entry.source).into(),
        source_character_id: entry.source_character_id.as_ref().map(|i| i.to_string()),
        source_character_name: entry
            .source_character_id
            .as_ref()
            .and_then(|id| instance_names.get(id))
            .cloned(),
        source_knowledge_id: entry.source_knowledge_id.as_ref().map(|id| id.to_string()),
        relay_chain_text: knowledge_relay_chain_text(entry, instance_names, knowledge_by_id),
        provenance_text: knowledge_provenance_text(entry, instance_names),
        turn_number: entry.turn_number,
        pinned: entry.pinned,
        propagation: propagation_policy_code(&entry.propagation),
    }
}

/// 任务 DTO（前端展示用）
#[derive(Debug, Clone, Serialize)]
pub struct StoryTaskDto {
    pub id: String,
    pub campaign_id: String,
    pub title: String,
    pub description: String,
    pub triggers: Vec<storyforge_domain::story_task::TaskTrigger>,
    pub status: storyforge_domain::story_task::TaskStatus,
    pub created_turn: u32,
    pub related_characters: Vec<String>,
    pub source: String,
    pub injected_turns: Vec<u32>,
}

impl From<&storyforge_domain::story_task::StoryTask> for StoryTaskDto {
    fn from(t: &storyforge_domain::story_task::StoryTask) -> Self {
        use storyforge_domain::story_task::TaskSource;
        let source = match t.source {
            TaskSource::UserPlanned => "user_planned",
            TaskSource::ExtractedFromNarrative => "from_narrative",
        };
        Self {
            id: t.id.to_string(),
            campaign_id: t.campaign_id.to_string(),
            title: t.title.clone(),
            description: t.description.clone(),
            triggers: t.triggers.clone(),
            status: t.status.clone(),
            created_turn: t.created_turn,
            related_characters: t.related_characters.iter().map(|i| i.to_string()).collect(),
            source: source.into(),
            injected_turns: t.injected_turns.clone(),
        }
    }
}

/// 本轮摘要 DTO
#[derive(Debug, Clone, Serialize)]
pub struct RoundSummaryDto {
    pub id: String,
    pub campaign_id: String,
    pub conversation_id: String,
    pub turn: u32,
    pub content: String,
    pub created_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub headline: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lineage_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub covered_by: Option<String>,
}

impl From<&storyforge_domain::agent::RoundSummary> for RoundSummaryDto {
    fn from(s: &storyforge_domain::agent::RoundSummary) -> Self {
        Self {
            id: s.id.to_string(),
            campaign_id: s.campaign_id.to_string(),
            conversation_id: s.conversation_id.to_string(),
            turn: s.turn,
            content: s.content.clone(),
            created_at: s.created_at.clone(),
            code: s.code.clone(),
            headline: s.headline.clone(),
            lineage_id: s.lineage_id.as_ref().map(|id| id.to_string()),
            covered_by: s.covered_by.as_ref().map(|id| id.to_string()),
        }
    }
}

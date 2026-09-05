use super::import_export::CampaignBundle;
use crate::error::TauriCommandError;
use crate::{CharacterInfo, storage::StoredCharacter};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use storyforge_domain::{
    Id, conversation::Conversation, turn::TurnRecord, world_info::WorldInfoBook,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct BundleRuntime {
    pub conversation: Conversation,
    pub character: StoredCharacter,
    pub world_info: WorldInfoBook,
    pub turns: Vec<TurnRecord>,
}

pub(crate) fn validate_runtime(bundle: &CampaignBundle) -> Result<(), TauriCommandError> {
    let Some(runtime) = &bundle.runtime else {
        if bundle.format_version >= 3 {
            return Err(TauriCommandError::validation("v3 Bundle 缺少正文快照"));
        }
        return Ok(());
    };
    let conv = &runtime.conversation;
    if bundle.campaign.conversation_id.as_ref() != Some(&conv.id)
        || conv.campaign_id.as_ref() != Some(&bundle.campaign.id)
        || bundle.campaign.pending_compress_publication.is_some()
        || bundle.card.as_ref().is_none_or(|card| {
            card.id != bundle.campaign.card_id
                || runtime.character.info.source_character_id.as_deref()
                    != Some(card.source_character_id.as_str())
        })
    {
        return Err(TauriCommandError::validation(
            "Bundle 正文或源角色卡范围不匹配",
        ));
    }
    let card = bundle.card.as_ref().expect("card scope checked");
    let mut ids = HashSet::new();
    if card
        .character_definitions
        .iter()
        .any(|definition| definition.card_id != card.id || !ids.insert(definition.id.clone()))
    {
        return Err(TauriCommandError::validation(
            "Bundle 角色定义范围无效或重复",
        ));
    }
    ids.clear();
    if bundle.instances.iter().any(|instance| {
        instance.campaign_id != bundle.campaign.id || !ids.insert(instance.id.clone())
    }) {
        return Err(TauriCommandError::validation("Bundle 实例范围无效或重复"));
    }
    ids.clear();
    if bundle
        .knowledge
        .iter()
        .any(|entry| entry.campaign_id != bundle.campaign.id || !ids.insert(entry.id.clone()))
    {
        return Err(TauriCommandError::validation("Bundle 知识范围无效或重复"));
    }
    ids.clear();
    if bundle
        .tasks
        .iter()
        .any(|task| task.campaign_id != bundle.campaign.id || !ids.insert(task.id.clone()))
    {
        return Err(TauriCommandError::validation("Bundle 任务范围无效或重复"));
    }
    let mut node_ids = HashSet::new();
    let mut variant_ids = HashSet::new();
    for node in &conv.nodes {
        if !node_ids.insert(node.id.clone())
            || node
                .parent_id
                .as_ref()
                .is_some_and(|parent| !node_ids.contains(parent) || parent == &node.id)
            || node.variants.is_empty()
            || node
                .variants
                .iter()
                .any(|v| !variant_ids.insert(v.id.clone()))
        {
            return Err(TauriCommandError::validation(
                "Bundle 消息图包含无效或重复引用",
            ));
        }
    }
    let mut turns = HashSet::new();
    let mut attempts = HashSet::new();
    for turn in &runtime.turns {
        if turn.campaign_id != bundle.campaign.id
            || turn.conversation_id != conv.id
            || turn.status.is_active()
            || !turns.insert(turn.turn_id.clone())
            || turn
                .accepted_attempt_id
                .as_ref()
                .is_some_and(|id| !turn.attempts.iter().any(|a| &a.attempt_id == id))
            || turn
                .attempts
                .iter()
                .any(|a| !attempts.insert(a.attempt_id.clone()))
        {
            return Err(TauriCommandError::validation("Bundle 轮次未结束或范围无效"));
        }
        if matches!(
            turn.status,
            storyforge_domain::turn::TurnStatus::Committed
                | storyforge_domain::turn::TurnStatus::Degraded
        ) && (!node_ids.contains(&turn.input_node_id)
            || turn.attempts.iter().any(|a| {
                a.status == storyforge_domain::turn::AttemptStatus::Committed
                    && !node_ids.contains(&a.variant_id)
            }))
        {
            return Err(TauriCommandError::validation(
                "Bundle 已采纳轮次缺少对应正文",
            ));
        }
    }
    Ok(())
}

/// Only rewrite identity fields in structured history. Never rewrite prose,
/// arbitrary variable values, raw card JSON, or rendered assets.
fn remap_history(value: &mut serde_json::Value, ids: &HashMap<Id, Id>, field: &str) {
    match value {
        serde_json::Value::String(text)
            if field == "id"
                || field.ends_with("_id")
                || matches!(field, "covers" | "related_characters" | "focalizers") =>
        {
            if let Some(id) = ids.get(&Id::from_str(text.as_str())) {
                *text = id.to_string();
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                remap_history(item, ids, field);
            }
        }
        serde_json::Value::Object(fields) => {
            for (key, item) in fields {
                if !matches!(key.as_str(), "value" | "raw_card_json" | "extensions") {
                    remap_history(item, ids, key);
                }
            }
        }
        _ => {}
    }
}

pub(crate) fn rewrite_runtime(
    mut runtime: BundleRuntime,
    ids: &mut HashMap<Id, Id>,
    source_character_id: &Id,
    lineage_id: Option<&Id>,
) -> Result<BundleRuntime, TauriCommandError> {
    for node in &runtime.conversation.nodes {
        ids.entry(node.id.clone()).or_default();
        for variant in &node.variants {
            ids.entry(variant.id.clone()).or_default();
            if let Some(provenance) = &variant.provenance {
                ids.entry(provenance.session_id.clone()).or_default();
            }
        }
    }
    for turn in &runtime.turns {
        ids.entry(turn.turn_id.clone()).or_default();
        ids.entry(turn.input_node_id.clone()).or_default();
        for attempt in &turn.attempts {
            ids.entry(attempt.attempt_id.clone()).or_default();
            ids.entry(attempt.variant_id.clone()).or_default();
            if let Some(batch) = &attempt.pending_state_changes {
                ids.entry(batch.commit_id.clone()).or_default();
            }
            if let Some(provenance) = &attempt.provenance {
                ids.entry(provenance.session_id.clone()).or_default();
            }
        }
    }
    let mut history = serde_json::json!({
        "conversation": runtime.conversation, "turns": runtime.turns,
    });
    remap_history(&mut history, ids, "");
    runtime.conversation = serde_json::from_value(history["conversation"].take())
        .map_err(|e| TauriCommandError::validation(e.to_string()))?;
    runtime.turns = serde_json::from_value(history["turns"].take())
        .map_err(|e| TauriCommandError::validation(e.to_string()))?;
    runtime.conversation.archived_upto = 0; // Vector indexes are rebuilt, not transferred.
    runtime.character.id = Id::new().to_string();
    runtime.character.info.source_character_id = Some(source_character_id.to_string());
    runtime.conversation.character_id = Some(runtime.character.id.clone());
    // Terminal journals are audit records, never replayable source-store jobs.
    for turn in &mut runtime.turns {
        for attempt in &mut turn.attempts {
            if let Some(batch) = &mut attempt.pending_state_changes {
                for mutation in &mut batch.mutations {
                    if let storyforge_domain::turn::Mutation::UpsertSummary(summary) = mutation {
                        summary.lineage_id = lineage_id.cloned();
                    }
                }
            }
        }
    }
    Ok(runtime)
}

pub(crate) fn legacy_source(card: &storyforge_domain::character::CharacterCard) -> StoredCharacter {
    let character = storyforge_domain::character::Character {
        id: card.source_character_id.clone(),
        name: card.name.clone(),
        description: String::new(),
        personality: String::new(),
        scenario: String::new(),
        first_mes: String::new(),
        mes_example: String::new(),
        system_prompt: String::new(),
        post_history_instructions: String::new(),
        alternate_greetings: vec![],
        tags: vec![],
        creator: String::new(),
        character_version: String::new(),
        spec_version: "3.0".into(),
        embedded_world_info: None,
        renderable_assets: None,
        extensions: serde_json::json!({}),
        raw_card_json: card.raw_card_json.clone(),
        source: storyforge_domain::Source::Native,
    };
    let character = serde_json::to_vec(&card.raw_card_json)
        .ok()
        .and_then(|bytes| storyforge_infra_import::import_character(&bytes).ok())
        .unwrap_or(character);
    let mut info = CharacterInfo::from(&character);
    info.source_character_id = Some(card.source_character_id.to_string());
    StoredCharacter {
        id: Id::new().to_string(),
        info,
        imported_at: chrono::Utc::now().to_rfc3339(),
    }
}

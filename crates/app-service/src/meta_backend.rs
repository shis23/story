//! Storage-boundary helpers for Meta commands.
//!
//! Read-only Meta health can use the opt-in SQLite authority today. Typed
//! patch persistence remains JSON-only until it has a single SQLite
//! transaction/UoW; callers must reject that path explicitly.

use storyforge_app_meta::{CampaignHealthSnapshot, HealthIssue, check_campaign_health};
use storyforge_domain::Id;

use crate::campaign_store::StoredCard;
use crate::sqlite_runtime;

/// Run the deterministic campaign health check from the live SQLite
/// authority. Invalid or mismatched stored payloads fail closed instead of
/// being treated as an empty card/definition set.
pub fn sqlite_campaign_health_issues(campaign_id: &Id) -> Result<Vec<HealthIssue>, String> {
    let campaign = sqlite_runtime::get_campaign(campaign_id)?
        .ok_or_else(|| format!("Campaign does not exist: {campaign_id}"))?;
    let payload = sqlite_runtime::get_card_payload(&campaign.card_id)?
        .ok_or_else(|| format!("Campaign card does not exist: {}", campaign.card_id))?;
    let stored: StoredCard = serde_json::from_value(payload)
        .map_err(|error| format!("invalid SQLite card payload: {error}"))?;

    if stored.card.id != campaign.card_id {
        return Err("SQLite campaign/card identity mismatch".into());
    }
    if stored
        .card
        .character_definitions
        .iter()
        .any(|definition| definition.card_id != campaign.card_id)
    {
        return Err("SQLite definition/card identity mismatch".into());
    }

    let instances = sqlite_runtime::list_instances(campaign_id)?;
    if instances
        .iter()
        .any(|instance| &instance.campaign_id != campaign_id)
    {
        return Err("SQLite instance/campaign scope mismatch".into());
    }
    let knowledge = sqlite_runtime::list_knowledge(campaign_id)?;
    if knowledge
        .iter()
        .any(|entry| &entry.campaign_id != campaign_id)
    {
        return Err("SQLite knowledge/campaign scope mismatch".into());
    }
    let tasks = sqlite_runtime::list_tasks(campaign_id)?;
    if tasks.iter().any(|task| &task.campaign_id != campaign_id) {
        return Err("SQLite task/campaign scope mismatch".into());
    }

    let snapshot = CampaignHealthSnapshot {
        instances: &instances,
        definitions: &stored.card.character_definitions,
        knowledge: &knowledge,
        tasks: &tasks,
    };
    Ok(check_campaign_health(&snapshot))
}

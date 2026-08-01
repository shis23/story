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

pub(crate) fn knowledge_source_code(
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

pub(crate) fn knowledge_provenance_text(
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

pub(crate) fn knowledge_actor_label(
    entry: &storyforge_domain::character_knowledge::CharacterKnowledgeEntry,
    instance_names: &std::collections::HashMap<Id, String>,
) -> String {
    instance_names
        .get(&entry.character_id)
        .cloned()
        .unwrap_or_else(|| entry.character_id.to_string())
}

pub(crate) fn knowledge_relay_chain_text(
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

pub(crate) fn propagation_policy_code(
    policy: &storyforge_domain::character_knowledge::PropagationPolicy,
) -> String {
    use storyforge_domain::character_knowledge::PropagationPolicy;
    match policy {
        PropagationPolicy::Open => "open".into(),
        PropagationPolicy::Private => "private".into(),
        PropagationPolicy::GroupRestricted(group) => format!("group:{group}"),
    }
}

pub(crate) fn knowledge_entry_to_dto(
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

/// 列出某 campaign 下某角色的可见信息（character_knowledge）
///
/// 不传 character_id 则返回整个 campaign 所有角色的知识。
#[tauri::command]
pub(crate) fn list_character_knowledge(
    campaign_id: String,
    character_id: Option<String>,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<Vec<KnowledgeEntryDto>, TauriCommandError> {
    let camp = Id::from_str(&campaign_id);
    let all_entries = state
        .storage()
        .list_knowledge(&camp)
        .map_err(TauriCommandError::storage)?;
    let entries: Vec<_> = if let Some(cid) = character_id {
        let cid = Id::from_str(&cid);
        all_entries
            .iter()
            .filter(|entry| entry.character_id == cid)
            .collect()
    } else {
        all_entries.iter().collect()
    };
    let instance_names: std::collections::HashMap<Id, String> = state
        .storage()
        .list_instances(&camp)
        .map_err(TauriCommandError::storage)?
        .into_iter()
        .map(|inst| (inst.id, inst.name))
        .collect();
    let knowledge_by_id: std::collections::HashMap<Id, _> = all_entries
        .iter()
        .map(|entry| (entry.id.clone(), entry))
        .collect();
    Ok(entries
        .into_iter()
        .map(|entry| knowledge_entry_to_dto(entry, &instance_names, &knowledge_by_id))
        .collect())
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

/// 列出某 campaign 的所有任务（可按状态筛：pending/active/likely_completed/completed/abandoned）
#[tauri::command]
pub(crate) fn list_tasks(
    campaign_id: String,
    status_filter: Option<String>,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<Vec<StoryTaskDto>, TauriCommandError> {
    let camp = Id::from_str(&campaign_id);
    let mut tasks = state
        .storage()
        .list_tasks(&camp)
        .map_err(TauriCommandError::storage)?;
    if let Some(filter) = status_filter {
        tasks.retain(|t| {
            let s = serde_json::to_string(&t.status).unwrap_or_default();
            // TaskStatus 序列化为 "pending"/"active"/{"likely_completed":...}/"completed"/"abandoned"
            s.starts_with(&format!("\"{filter}"))
                || s.starts_with('{') && filter == "likely_completed"
        });
    }
    Ok(tasks.iter().map(StoryTaskDto::from).collect())
}

/// 创建任务（前端 UI：用户手动规划伏笔/目标）
#[tauri::command]
pub fn create_task(
    campaign_id: String,
    title: String,
    description: String,
    triggers: Vec<storyforge_domain::story_task::TaskTrigger>,
    created_turn: Option<u32>,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<String, TauriCommandError> {
    if title.trim().is_empty() {
        return Err("任务标题不能为空".into());
    }
    // P0-7：活动 Turn 期间拒绝直接写任务
    reject_if_active_turn(state.storage(), &Id::from_str(&campaign_id))?;
    let task = storyforge_domain::story_task::StoryTask::user_planned(
        Id::from_str(&campaign_id),
        title,
        description,
        triggers,
        created_turn.unwrap_or(0),
    );
    let id = task.id.to_string();
    state
        .storage()
        .add_task(&task)
        .map_err(|e| TauriCommandError::storage(format!("创建任务失败: {e}")))?;
    Ok(id)
}

/// 标记任务完成（用户确认）
#[tauri::command]
pub fn complete_task(
    task_id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<(), TauriCommandError> {
    let mut task = state
        .storage()
        .get_task(&Id::from_str(&task_id))
        .map_err(TauriCommandError::storage)?
        .ok_or_else(|| TauriCommandError::not_found(format!("找不到任务 {task_id}")))?;
    // P0-7：活动 Turn 期间拒绝直接写任务
    reject_if_active_turn(state.storage(), &task.campaign_id)?;
    task.complete();
    state
        .storage()
        .update_task(&task)
        .map_err(|e| TauriCommandError::storage(format!("完成任务失败: {e}")))?;
    Ok(())
}

/// 放弃任务
#[tauri::command]
pub fn abandon_task(
    task_id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<(), TauriCommandError> {
    let mut task = state
        .storage()
        .get_task(&Id::from_str(&task_id))
        .map_err(TauriCommandError::storage)?
        .ok_or_else(|| TauriCommandError::not_found(format!("找不到任务 {task_id}")))?;
    // P0-7：活动 Turn 期间拒绝直接写任务
    reject_if_active_turn(state.storage(), &task.campaign_id)?;
    task.abandon();
    state
        .storage()
        .update_task(&task)
        .map_err(|e| TauriCommandError::storage(format!("放弃任务失败: {e}")))?;
    Ok(())
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

/// 列出某 campaign 的所有本轮剧情摘要（按 turn 升序）
#[tauri::command]
pub(crate) fn list_round_summaries(
    campaign_id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<Vec<RoundSummaryDto>, TauriCommandError> {
    let camp = Id::from_str(&campaign_id);
    Ok(state
        .storage()
        .list_summaries(&camp)
        .map_err(TauriCommandError::storage)?
        .iter()
        .map(RoundSummaryDto::from)
        .collect())
}

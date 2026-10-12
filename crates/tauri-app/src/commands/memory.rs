use super::super::*;

pub use storyforge_app_service::services::memory::*;

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
    // 三.5：活动 Turn 屏障 + 任务写入在同一原子单元内完成。
    let campaign_id = Id::from_str(&campaign_id);
    let task = storyforge_domain::story_task::StoryTask::user_planned(
        campaign_id.clone(),
        title,
        description,
        triggers,
        created_turn.unwrap_or(0),
    );
    let id = task.id.to_string();
    state
        .storage()
        .add_idle_task(&campaign_id, &task)
        .map_err(|e| TauriCommandError::storage(format!("创建任务失败: {e}")))?;
    Ok(id)
}

/// 标记任务完成（用户确认）
#[tauri::command]
pub fn complete_task(
    task_id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<(), TauriCommandError> {
    // 三.5：活动 Turn 屏障（按任务所属 campaign）+ 读改写在同一原子单元内。
    let task_id = Id::from_str(&task_id);
    let applied = state
        .storage()
        .mutate_idle_task(&task_id, |task| {
            task.complete();
            Ok(())
        })
        .map_err(|e| TauriCommandError::storage(format!("完成任务失败: {e}")))?;
    if applied.is_none() {
        return Err(TauriCommandError::not_found(format!(
            "找不到任务 {task_id}"
        )));
    }
    Ok(())
}

/// 放弃任务
#[tauri::command]
pub fn abandon_task(
    task_id: String,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<(), TauriCommandError> {
    // 三.5：活动 Turn 屏障（按任务所属 campaign）+ 读改写在同一原子单元内。
    let task_id = Id::from_str(&task_id);
    let applied = state
        .storage()
        .mutate_idle_task(&task_id, |task| {
            task.abandon();
            Ok(())
        })
        .map_err(|e| TauriCommandError::storage(format!("放弃任务失败: {e}")))?;
    if applied.is_none() {
        return Err(TauriCommandError::not_found(format!(
            "找不到任务 {task_id}"
        )));
    }
    Ok(())
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

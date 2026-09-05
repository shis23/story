use crate::Id;
use crate::conversation::{Conversation, Role, VariantStatus};
use crate::turn::{AttemptStatus, TurnRecord, TurnStatus};

/// Prepare a history deletion without changing either durable authority.
/// Callers must serialize this snapshot check and both writes.
pub fn truncate_uncommitted(
    conversation: &Conversation,
    turns: &[TurnRecord],
    node_id: &Id,
) -> Result<(Conversation, Vec<TurnRecord>), String> {
    let mut cut = conversation
        .nodes
        .iter()
        .position(|n| &n.id == node_id)
        .ok_or_else(|| "消息不存在".to_string())?;
    for turn in turns {
        if turn.conversation_id != conversation.id
            || conversation.campaign_id.as_ref() != Some(&turn.campaign_id)
        {
            return Err("轮次与对话范围不匹配".into());
        }
        if matches!(turn.status, TurnStatus::Generating | TurnStatus::Committing) {
            return Err("轮次正在生成或提交，请等待结束后再删除".into());
        }
    }
    let removed = &conversation.nodes[cut..];
    let affected: Vec<_> = turns
        .iter()
        .filter(|turn| {
            removed.iter().any(|node| {
                node.id == turn.input_node_id
                    || turn
                        .attempts
                        .iter()
                        .any(|attempt| attempt.variant_id == node.id)
            })
        })
        .collect();
    for turn in &affected {
        if matches!(turn.status, TurnStatus::Committed | TurnStatus::Degraded)
            || turn
                .attempts
                .iter()
                .any(|a| a.status == AttemptStatus::Committed)
        {
            return Err("已采纳历史不能直接删除，请保留原局或从已提交末尾创建分支".into());
        }
        if let Some(input) = conversation
            .nodes
            .iter()
            .position(|n| n.id == turn.input_node_id)
        {
            cut = cut.min(input);
        }
    }
    if conversation.campaign_id.is_some()
        && conversation.nodes[cut..].iter().any(|node| {
            node.variants
                .iter()
                .any(|v| v.role == Role::Assistant && v.status == VariantStatus::Final)
        })
    {
        return Err("已采纳正文不能直接删除".into());
    }
    let mut changed_turns = Vec::new();
    for turn in affected {
        let mut turn = turn.clone();
        turn.status = TurnStatus::Abandoned;
        for attempt in &mut turn.attempts {
            attempt.status = AttemptStatus::Discarded;
            attempt.pending_state_changes = None;
            attempt.pending_temporary_instances.clear();
        }
        turn.touch();
        changed_turns.push(turn);
    }
    let mut updated = conversation.clone();
    updated.nodes.truncate(cut);
    updated.archived_upto = updated.archived_upto.min(
        updated
            .nodes
            .iter()
            .filter(|n| {
                n.active()
                    .is_some_and(|v| v.status != VariantStatus::Discarded)
            })
            .count(),
    );
    updated.updated_at = chrono::Utc::now();
    Ok((updated, changed_turns))
}

pub fn require_committed_head(conversation: &Conversation, node_id: &Id) -> Result<(), String> {
    let head = conversation
        .nodes
        .iter()
        .rev()
        .find(|n| {
            n.active()
                .is_some_and(|v| v.status != VariantStatus::Discarded)
        })
        .ok_or_else(|| "对话没有可分支的已采纳正文".to_string())?;
    if &head.id != node_id
        || !head
            .active()
            .is_some_and(|v| v.role == Role::Assistant && v.status == VariantStatus::Final)
    {
        return Err("目前仅支持从已采纳的末尾正文创建分支".into());
    }
    Ok(())
}

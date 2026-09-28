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

// ─── 测试（D-18：本文件此前完全没有测试）──────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conversation::{MessageNode, MessageVariant};
    use crate::turn::TurnAttempt;
    use chrono::Utc;

    fn node(id: &str, role: Role, content: &str, status: VariantStatus) -> MessageNode {
        MessageNode {
            id: Id::from_str(id),
            parent_id: None,
            variants: vec![MessageVariant {
                id: Id::new(),
                role,
                content: content.into(),
                created_at: Utc::now(),
                status,
                provenance: None,
            }],
            active_variant: 0,
        }
    }

    fn conv(campaign_id: Option<Id>, nodes: Vec<MessageNode>) -> Conversation {
        let mut conversation = Conversation::new(Some("char1".into()), campaign_id);
        conversation.id = Id::from_str("c1");
        conversation.nodes = nodes;
        conversation
    }

    fn attempt(variant_id: &str, status: AttemptStatus) -> TurnAttempt {
        TurnAttempt {
            attempt_id: Id::new(),
            variant_id: Id::from_str(variant_id),
            draft_hash: "hash".into(),
            status,
            pending_state_changes: Some(crate::turn::MutationBatch::new(Id::new(), 0)),
            derivation: None,
            quality_report: None,
            // 只有 accept 时才会落库；删除路径必须清空它们
            pending_temporary_instances: vec![crate::campaign::CharacterInstance::temporary(
                Id::new(),
                "临时角色",
            )],
            provenance: None,
            created_at: Utc::now().to_rfc3339(),
        }
    }

    fn turn(
        conversation_id: &Id,
        campaign_id: &Id,
        input_node_id: &str,
        status: TurnStatus,
        attempts: Vec<TurnAttempt>,
    ) -> TurnRecord {
        let mut record = TurnRecord::new(
            campaign_id.clone(),
            conversation_id.clone(),
            Id::from_str(input_node_id),
            0,
        );
        record.status = status;
        record.attempts = attempts;
        record
    }

    #[test]
    fn unknown_node_id_is_rejected() {
        let campaign = Id::new();
        let conversation = conv(Some(campaign.clone()), vec![]);
        let err = truncate_uncommitted(&conversation, &[], &Id::from_str("missing"))
            .expect_err("未知节点必须报错");
        assert!(err.contains("消息不存在"), "{err}");
    }

    #[test]
    fn out_of_scope_turns_are_rejected() {
        let campaign = Id::new();
        let conversation = conv(
            Some(campaign.clone()),
            vec![node("n1", Role::User, "u1", VariantStatus::Final)],
        );
        // conversation_id 不匹配
        let foreign = turn(&Id::new(), &campaign, "n1", TurnStatus::Failed, vec![]);
        assert!(
            truncate_uncommitted(&conversation, &[foreign], &Id::from_str("n1"))
                .expect_err("对话范围不匹配必须报错")
                .contains("轮次与对话范围不匹配")
        );

        // campaign_id 不匹配
        let other = turn(
            &conversation.id.clone(),
            &Id::new(),
            "n1",
            TurnStatus::Failed,
            vec![],
        );
        assert!(truncate_uncommitted(&conversation, &[other], &Id::from_str("n1")).is_err());
    }

    #[test]
    fn in_flight_turns_block_deletion() {
        let campaign = Id::new();
        let conversation = conv(
            Some(campaign.clone()),
            vec![
                node("n1", Role::User, "u1", VariantStatus::Final),
                node("n2", Role::Assistant, "草稿", VariantStatus::Draft),
            ],
        );
        for status in [TurnStatus::Generating, TurnStatus::Committing] {
            let record = turn(
                &conversation.id.clone(),
                &campaign,
                "n1",
                status.clone(),
                vec![],
            );
            let err = truncate_uncommitted(&conversation, &[record], &Id::from_str("n2"))
                .expect_err("生成/提交中的轮次必须阻止删除");
            assert!(err.contains("正在生成或提交"), "{status:?}: {err}");
        }
    }

    #[test]
    fn committed_body_in_removed_range_cannot_be_deleted() {
        let campaign = Id::new();
        let conversation = conv(
            Some(campaign.clone()),
            vec![
                node("n1", Role::User, "u1", VariantStatus::Final),
                node("n2", Role::Assistant, "已采纳正文", VariantStatus::Final),
            ],
        );
        let err = truncate_uncommitted(&conversation, &[], &Id::from_str("n1"))
            .expect_err("已采纳正文不能直接删除");
        assert!(err.contains("已采纳正文"), "{err}");
    }

    #[test]
    fn committed_turn_in_removed_range_cannot_be_deleted() {
        // removed 区间没有 Final 正文（只有未采纳草稿），因此只有轮次状态能拦住它
        let campaign = Id::new();
        let conversation = conv(
            Some(campaign.clone()),
            vec![
                node("n1", Role::User, "u1", VariantStatus::Final),
                node("n2", Role::Assistant, "未采纳草稿", VariantStatus::Draft),
            ],
        );
        let record = turn(
            &conversation.id.clone(),
            &campaign,
            "n1",
            TurnStatus::Committed,
            vec![attempt("n2", AttemptStatus::Committed)],
        );
        let err = truncate_uncommitted(&conversation, &[record], &Id::from_str("n2"))
            .expect_err("已采纳历史必须报错");
        assert!(err.contains("已采纳历史"), "{err}");

        // Degraded / 已提交 attempt 同样拦截
        let degraded = turn(
            &conversation.id.clone(),
            &campaign,
            "n1",
            TurnStatus::Degraded,
            vec![attempt("n2", AttemptStatus::DraftReady)],
        );
        assert!(
            truncate_uncommitted(&conversation, &[degraded], &Id::from_str("n2"))
                .expect_err("Degraded 轮次必须报错")
                .contains("已采纳历史")
        );
        let committed_attempt = turn(
            &conversation.id.clone(),
            &campaign,
            "n1",
            TurnStatus::AwaitingAcceptance,
            vec![attempt("n2", AttemptStatus::Committed)],
        );
        assert!(
            truncate_uncommitted(&conversation, &[committed_attempt], &Id::from_str("n2"))
                .expect_err("含已提交 attempt 的轮次必须报错")
                .contains("已采纳历史")
        );
    }

    #[test]
    fn uncommitted_tail_is_truncated_and_turns_abandoned() {
        let campaign = Id::new();
        let conversation = conv(
            Some(campaign.clone()),
            vec![
                node("n1", Role::User, "u1", VariantStatus::Final),
                node("n2", Role::Assistant, "未采纳草稿", VariantStatus::Draft),
            ],
        );
        let record = turn(
            &conversation.id.clone(),
            &campaign,
            "n1",
            TurnStatus::Failed,
            vec![attempt("n2", AttemptStatus::DraftReady)],
        );

        let (updated, changed) =
            truncate_uncommitted(&conversation, &[record], &Id::from_str("n2"))
                .expect("应允许删除");

        // cut 前移到 input_node（n1 位置 0）→ 两个节点都被移除
        assert!(updated.nodes.is_empty(), "cut 必须前移到 input_node");
        assert_eq!(changed.len(), 1);
        assert_eq!(changed[0].status, TurnStatus::Abandoned);
        assert!(changed[0].attempts.iter().all(|a| {
            a.status == AttemptStatus::Discarded
                && a.pending_state_changes.is_none()
                && a.pending_temporary_instances.is_empty()
        }));
        // 原对话不被就地修改
        assert_eq!(conversation.nodes.len(), 2);
    }

    #[test]
    fn tail_truncation_keeps_earlier_nodes_and_clamps_watermark() {
        let campaign = Id::new();
        let mut conversation = conv(
            Some(campaign.clone()),
            vec![
                node("n1", Role::User, "u1", VariantStatus::Final),
                node("n2", Role::Assistant, "已采纳", VariantStatus::Final),
                node("n3", Role::User, "u2", VariantStatus::Draft),
            ],
        );
        // 水位名高于实际节点数：删除后必须钳回活跃节点数
        conversation.archived_upto = 99;
        let (updated, changed) =
            truncate_uncommitted(&conversation, &[], &Id::from_str("n3")).expect("应允许删除");
        assert_eq!(updated.nodes.len(), 2);
        assert_eq!(changed.len(), 0, "无受影响轮次时不应产生变更记录");
        assert!(updated.archived_upto <= 2);
    }

    #[test]
    fn require_committed_head_only_accepts_accepted_tail() {
        let campaign = Id::new();
        let conversation = conv(
            Some(campaign),
            vec![
                node("n1", Role::User, "u1", VariantStatus::Final),
                node("n2", Role::Assistant, "已采纳正文", VariantStatus::Final),
            ],
        );
        require_committed_head(&conversation, &Id::from_str("n2")).expect("末尾已采纳正文可分支");

        // 非末尾节点
        assert!(require_committed_head(&conversation, &Id::from_str("n1")).is_err());
        // 末尾是 user 消息（不是 Final assistant）
        let user_tail = conv(
            None,
            vec![node("n1", Role::User, "u1", VariantStatus::Final)],
        );
        assert!(require_committed_head(&user_tail, &Id::from_str("n1")).is_err());
        // 没有任何活跃节点
        let empty = conv(None, vec![]);
        assert!(
            require_committed_head(&empty, &Id::from_str("n1"))
                .expect_err("空对话不能分支")
                .contains("没有可分支")
        );
        // 末尾变体被 Discarded → 不应被当作 head
        let discarded_tail = conv(
            None,
            vec![node(
                "n1",
                Role::Assistant,
                "已丢弃",
                VariantStatus::Discarded,
            )],
        );
        assert!(require_committed_head(&discarded_tail, &Id::from_str("n1")).is_err());
    }
}

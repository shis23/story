use super::*;
use crate::commands::writing_regenerate::validate_regenerate_campaign_scope;

#[test]
fn sync_attempt_after_autofix_updates_hash_to_final_text() {
    let original = "原稿含破折号——且过短";
    let fixed = "修复后的正文足够长，并且去掉了破折号，急诊灯下林秋与陈警官对坐，雨声敲窗，空气里有消毒水味。";
    let mut attempt = storyforge_domain::turn::TurnAttempt {
        attempt_id: Id::new(),
        variant_id: Id::new(),
        draft_hash: turn_lifecycle::compute_draft_hash(original),
        status: storyforge_domain::turn::AttemptStatus::DraftReady,
        pending_state_changes: None,
        derivation: None,
        quality_report: None,
        pending_temporary_instances: vec![],
        provenance: None,
        created_at: chrono::Utc::now().to_rfc3339(),
    };
    let report = storyforge_domain::turn::QualityReport { warnings: vec![] };
    turn_lifecycle::sync_attempt_after_autofix(&mut attempt, fixed, report);
    assert_eq!(
        attempt.draft_hash,
        turn_lifecycle::compute_draft_hash(fixed),
        "auto-fix 后 Attempt.draft_hash 必须等于修复稿 hash"
    );
    assert_ne!(
        attempt.draft_hash,
        turn_lifecycle::compute_draft_hash(original),
        "不能继续指向原稿 hash"
    );
    assert!(attempt.quality_report.is_some());
}

#[test]
fn autofix_response_must_prefer_fixed_over_original() {
    // 直接测生产 helper，防止命令再次回退到原稿
    let original = "原稿".to_string();
    let fixed = "修复稿".to_string();
    assert_eq!(
        turn_lifecycle::prefer_autofix_response_text(Some(fixed.clone()), original.clone()),
        fixed
    );
    assert_eq!(
        turn_lifecycle::prefer_autofix_response_text(None, original.clone()),
        original
    );
}

#[test]
fn regenerate_scope_rejects_cross_campaign_before_pipeline_mutates_a_draft() {
    let active_campaign = Id::from_str("campaign-active");
    let foreign_campaign = Id::from_str("campaign-foreign");
    let foreign_conversation = Conversation::new(None, Some(foreign_campaign));
    let active_turn = storyforge_domain::turn::TurnRecord::new(
        active_campaign.clone(),
        Id::from_str("conversation-active"),
        Id::from_str("input-active"),
        0,
    );

    assert!(
        validate_regenerate_campaign_scope(
            Some(&active_campaign),
            &foreign_conversation,
            Some(&active_turn),
        )
        .is_err(),
        "scope validation must reject before PipelineOrchestrator can alter the foreign draft"
    );

    let matching_conversation = Conversation::new(None, Some(active_campaign.clone()));
    let matching_turn = storyforge_domain::turn::TurnRecord::new(
        active_campaign.clone(),
        matching_conversation.id.clone(),
        Id::from_str("input-matching"),
        0,
    );
    assert!(
        validate_regenerate_campaign_scope(
            Some(&active_campaign),
            &matching_conversation,
            Some(&matching_turn),
        )
        .is_ok()
    );
}

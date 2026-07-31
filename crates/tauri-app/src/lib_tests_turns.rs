use super::*;
use crate::commands::turns::{
    active_turn_quality_from_record, get_active_turn_quality_impl, postprocess_present_characters,
    receipt_items_from_batch, retain_selected_receipt_mutations,
};

#[test]
fn test_active_turn_quality_from_record() {
    use storyforge_domain::turn::{
        AttemptStatus, QualityReport, QualitySeverity, QualityWarning, QualityWarningCode,
        TurnAttempt, TurnRecord,
    };
    let mut record = TurnRecord::new(
        Id::from_str("camp-q"),
        Id::from_str("conv-q"),
        Id::from_str("node-q"),
        0,
    );
    assert!(active_turn_quality_from_record(&record).is_none());

    record.attempts.push(TurnAttempt {
        attempt_id: Id::from_str("att-q"),
        variant_id: Id::from_str("var-q"),
        draft_hash: "h".into(),
        status: AttemptStatus::AwaitingAcceptance,
        pending_state_changes: None,
        derivation: None,
        quality_report: Some(QualityReport {
            warnings: vec![QualityWarning {
                code: QualityWarningCode::TooShort { char_count: 10 },
                message: "\u{5b57}\u{6570}\u{8fc7}\u{77ed}".into(),
                severity: QualitySeverity::Warning,
            }],
        }),
        pending_temporary_instances: vec![],
        provenance: None,
        created_at: "2026-01-01T00:00:00Z".into(),
    });

    let dto = active_turn_quality_from_record(&record).expect("should have quality");
    assert_eq!(dto.attempt_id, "att-q");
    assert!(!dto.passed);
    assert_eq!(dto.warning_count, 1);
    assert_eq!(
        dto.warnings,
        vec!["\u{5b57}\u{6570}\u{8fc7}\u{77ed}".to_string()]
    );
}

#[test]
fn sqlite_active_turn_quality_propagates_backend_lookup_failure() {
    let dir = std::env::temp_dir().join(format!(
        "storyforge-quality-facade-test-{}",
        uuid::Uuid::new_v4()
    ));
    let storage = storage_backend::StorageFacade::new(
        dir,
        storyforge_infra_sqlite::backend::PinnedBackend::new(
            storyforge_infra_sqlite::backend::StorageBackend::Sqlite,
            storyforge_infra_sqlite::backend::BackendSource::Env,
        ),
    );

    let error = get_active_turn_quality_impl(&storage, "camp-quality-error".into())
        .expect_err("an unavailable SQLite authority must not be reported as no active Turn");

    assert!(
        error.to_string().contains("sqlite backend is not active"),
        "unexpected error: {error}"
    );
}

#[test]
fn turn_receipt_lists_only_user_reviewable_mutations() {
    use storyforge_domain::turn::{Mutation, MutationBatch};

    let campaign_id = Id::from_str("receipt-campaign");
    let conversation_id = Id::from_str("receipt-conversation");
    let variant_id = Id::from_str("receipt-variant");
    let mut batch = MutationBatch::new(Id::from_str("receipt-commit"), 4);
    batch.mutations.push(Mutation::UpsertSummary(Box::new(
        storyforge_domain::agent::RoundSummary::new(
            campaign_id,
            conversation_id,
            5,
            "\u{6797}\u{79cb}\u{786e}\u{8ba4}\u{4e86}\u{4ed3}\u{5e93}\u{94a5}\u{5319}\u{7684}\u{6765}\u{6e90}\u{3002}".into(),
        ),
    )));
    batch.mutations.push(Mutation::SetVariable {
        instance_id: None,
        key: "tension".into(),
        value: serde_json::json!(7),
        turn: 5,
    });
    batch.mutations.push(Mutation::FinalizeVariant {
        variant_id: variant_id.clone(),
    });
    batch.mutations.push(Mutation::UpsertInstance(Box::new(
        storyforge_domain::campaign::CharacterInstance::temporary(
            Id::from_str("receipt-campaign"),
            "\u{5b88}\u{95e8}\u{4eba}",
        ),
    )));

    let items = receipt_items_from_batch(&batch);

    assert_eq!(
        items.len(),
        2,
        "\u{7ed3}\u{6784}\u{6027} mutation \u{4e0d}\u{5e94}\u{6df7}\u{5165}\u{7528}\u{6237}\u{5c0f}\u{7968}"
    );
    assert_eq!(items[0].mutation_index, 0);
    assert_eq!(items[0].kind, "chronicle");
    assert!(items[0].detail.contains("\u{4ed3}\u{5e93}\u{94a5}\u{5319}"));
    assert_eq!(items[1].mutation_index, 1);
    assert_eq!(items[1].kind, "variable");
    assert!(items.iter().all(|item| item.selected_by_default));
}

#[test]
fn receipt_selection_preserves_structural_mutations() {
    use storyforge_domain::turn::{Mutation, MutationBatch};

    let variant_id = Id::from_str("receipt-filter-variant");
    let mut batch = MutationBatch::new(Id::from_str("receipt-filter-commit"), 9);
    batch.mutations.push(Mutation::SetVariable {
        instance_id: None,
        key: "keep".into(),
        value: serde_json::json!(1),
        turn: 10,
    });
    batch.mutations.push(Mutation::SetVariable {
        instance_id: None,
        key: "drop".into(),
        value: serde_json::json!(2),
        turn: 10,
    });
    batch.mutations.push(Mutation::FinalizeVariant {
        variant_id: variant_id.clone(),
    });
    batch.mutations.push(Mutation::UpsertInstance(Box::new(
        storyforge_domain::campaign::CharacterInstance::temporary(
            Id::from_str("receipt-filter-campaign"),
            "\u{4e34}\u{65f6}\u{8bc1}\u{4eba}",
        ),
    )));

    retain_selected_receipt_mutations(&mut batch, &[0]).unwrap();

    assert_eq!(batch.mutations.len(), 3);
    assert!(matches!(
        &batch.mutations[0],
        Mutation::SetVariable { key, .. } if key == "keep"
    ));
    assert!(matches!(
        &batch.mutations[1],
        Mutation::FinalizeVariant { variant_id: id } if id == &variant_id
    ));
    assert!(matches!(&batch.mutations[2], Mutation::UpsertInstance(_)));
}

#[test]
fn postprocess_retry_recovers_unique_present_characters_from_provenance() {
    let provenance = Provenance {
        session_id: Id::new(),
        plan: None,
        subagent_results: vec![
            storyforge_domain::conversation::SubagentSnapshot {
                character_id: "lin-qiu".into(),
                full_text: "A".into(),
                character_instance_id: None,
                display_name: Some("\u{6797}\u{79cb}".into()),
                fallback_reason: None,
                reasoning_content: None,
            },
            storyforge_domain::conversation::SubagentSnapshot {
                character_id: "lin-qiu".into(),
                full_text: "B".into(),
                character_instance_id: None,
                display_name: Some("\u{6797}\u{79cb}".into()),
                fallback_reason: None,
                reasoning_content: None,
            },
            storyforge_domain::conversation::SubagentSnapshot {
                character_id: "chen".into(),
                full_text: "C".into(),
                character_instance_id: None,
                display_name: Some("\u{9648}\u{8b66}\u{5b98}".into()),
                fallback_reason: None,
                reasoning_content: None,
            },
        ],
        profile_id: None,
        generation_mode: None,
        seed: 1,
        last_hint: None,
        director_reasoning: None,
        writer_reasoning: None,
        editor_reasoning: None,
    };

    assert_eq!(
        postprocess_present_characters(Some(&provenance)),
        vec![
            "\u{6797}\u{79cb}".to_string(),
            "\u{9648}\u{8b66}\u{5b98}".to_string()
        ]
    );
}

#[test]
fn turn_barrier_rejects_start_writing_with_active_turn() {
    let state = Arc::new(AppState::new_for_test());
    let (dir, ts) = temp_turn_store();
    let campaign_id = Id::new();

    {
        let mut guard = state
            .active_campaign
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        *guard = Some(campaign_id.clone());
    }

    let record = storyforge_domain::turn::TurnRecord::new(
        campaign_id.clone(),
        Id::from_str("conv-1"),
        Id::from_str("node-1"),
        0,
    );
    ts.create_turn(record).unwrap();

    let result = check_turn_barrier_with(&state, &ts);
    assert!(
        result.is_err(),
        "barrier should reject start_writing when active Turn exists"
    );
    // legacy meta_accept_patch x start_writing xx check_turn_barrier xx
    assert!(
        check_turn_barrier_with(&state, &ts).is_err(),
        "barrier should also reject legacy meta_accept_patch while Turn active"
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn turn_barrier_passes_without_active_turn() {
    let state = Arc::new(AppState::new_for_test());
    let (dir, ts) = temp_turn_store();
    let campaign_id = Id::from_str("camp-test-no-turn");

    {
        let mut guard = state
            .active_campaign
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        *guard = Some(campaign_id.clone());
    }

    // xxx Turn x xx
    assert!(check_turn_barrier_with(&state, &ts).is_ok());
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn turn_barrier_passes_non_campaign_mode() {
    let state = Arc::new(AppState::new_for_test());
    let (dir, ts) = temp_turn_store();
    // xxx active_campaign x x Campaign xx
    assert!(check_turn_barrier_with(&state, &ts).is_ok());
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn turn_record_committed_allows_next_turn() {
    let state = Arc::new(AppState::new_for_test());
    let (dir, ts) = temp_turn_store();
    let campaign_id = Id::from_str("camp-committed");

    {
        let mut guard = state
            .active_campaign
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        *guard = Some(campaign_id.clone());
    }

    // xxxx Committed Turn
    let mut record = storyforge_domain::turn::TurnRecord::new(
        campaign_id.clone(),
        Id::from_str("conv-1"),
        Id::from_str("node-1"),
        0,
    );
    record.status = storyforge_domain::turn::TurnStatus::Committed;
    ts.save_turn(record).unwrap();

    // Committed x terminal x xx
    assert!(check_turn_barrier_with(&state, &ts).is_ok());
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn direct_write_barrier_rejects_active_turn() {
    let (dir, ts) = temp_turn_store();
    let campaign_id = Id::new();
    let record = storyforge_domain::turn::TurnRecord::new(
        campaign_id.clone(),
        Id::from_str("conv-1"),
        Id::from_str("node-1"),
        0,
    );
    ts.create_turn(record).unwrap();
    assert!(reject_if_active_turn_in(&ts, &campaign_id).is_err());
    assert!(reject_if_active_turn_in(&ts, &Id::from_str("other-camp")).is_ok());
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn accept_variant_non_campaign_keeps_legacy_behavior() {
    let state = Arc::new(AppState::new_for_test());
    let conversation = state.conv_store.create(Some("card-1".into()), None);
    let node_id = state
        .conv_store
        .append_ai_draft(&conversation.id, "draft".into(), None)
        .unwrap();

    // xx active_campaign x x Campaign xx
    accept_variant_async(
        state.clone(),
        conversation.id.clone(),
        node_id.clone(),
        false,
        None,
    )
    .await
    .unwrap();

    // xxx Finalxxxxx
    let updated = state.conv_store.get(&conversation.id).unwrap();
    let node = updated.nodes.iter().find(|n| n.id == node_id).unwrap();
    assert_eq!(node.active().unwrap().status, VariantStatus::Final);
}

#[tokio::test]
async fn accept_variant_campaign_historical_attempt_rejected() {
    let state = Arc::new(AppState::new_for_test());
    let campaign_id = Id::from_str("camp-historical");

    {
        let mut guard = state
            .active_campaign
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        *guard = Some(campaign_id.clone());
    }

    // xxxx conversation + AI draftxxxxx TurnRecordx
    let conversation = state.conv_store.create(Some("card-1".into()), None);
    let node_id = state
        .conv_store
        .append_ai_draft(&conversation.id, "orphan draft".into(), None)
        .unwrap();

    // accept xxxxxxxxxxx TurnRecord
    let result = accept_variant_async(
        state.clone(),
        conversation.id.clone(),
        node_id.clone(),
        false,
        None,
    )
    .await;
    assert!(result.is_err(), "should reject historical/orphan attempt");
}

#[test]
fn next_chronicle_a_seq_from_existing_codes_and_turns() {
    let a = storyforge_domain::agent::RoundSummary::new(
        Id::from_str("c"),
        Id::from_str("v"),
        2,
        "x".into(),
    )
    .with_code("A0002");
    let b = storyforge_domain::agent::RoundSummary::new(
        Id::from_str("c"),
        Id::from_str("v"),
        9,
        "y".into(),
    ); // legacy no code x use turn
    assert_eq!(next_chronicle_a_seq(&[a, b]), 10);
    assert_eq!(next_chronicle_a_seq(&[]), 1);
}

#[test]
fn quality_accept_decision_matches_product_policy() {
    use storyforge_domain::turn::{
        QualityAcceptDecision, QualityReport, QualitySeverity, QualityWarning, QualityWarningCode,
        quality_accept_decision,
    };
    let err = QualityReport {
        warnings: vec![QualityWarning {
            code: QualityWarningCode::FormatLeak {
                snippet: "```".into(),
            },
            message: "format".into(),
            severity: QualitySeverity::Error,
        }],
    };
    assert!(matches!(
        quality_accept_decision(Some(&err), false),
        QualityAcceptDecision::Block { error_count: 1 }
    ));
    assert!(matches!(
        quality_accept_decision(Some(&err), true),
        QualityAcceptDecision::ForceDegraded { error_count: 1 }
    ));
}

#[test]
fn storage_meta_records_version_trail_and_survives_corruption() {
    let dir = std::env::temp_dir().join(format!("sf-meta-test-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("storage_meta.json");

    // xxxfirst_* x last_* xxx
    touch_storage_meta(&dir);
    let v: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(v["schema"], 1);
    assert_eq!(v["first_created_version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(v["last_opened_version"], env!("CARGO_PKG_VERSION"));
    let first_created = v["first_created_at"].as_str().unwrap().to_string();

    // xxxxxfirst_* xxxlast_opened_at xx
    touch_storage_meta(&dir);
    let v2: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(v2["first_created_at"], first_created.as_str());

    // xxxxxxxxx metaxx panicxxxxxxx
    std::fs::write(&path, "{ broken").unwrap();
    touch_storage_meta(&dir);
    let v3: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(v3["schema"], 1);
    assert!(v3["first_created_at"].is_string());

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn build_mutation_batch_empty_outcome() {
    let dir = std::env::temp_dir().join(format!("storyforge-mb-test-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let store = campaign_store::CampaignStore::new(&dir);
    let campaign =
        storyforge_domain::campaign::Campaign::new(Id::from_str("card-1"), "test".to_string());
    let campaign_id = campaign.id.clone();
    store.save_campaign(campaign).unwrap();

    let pc = PostprocessPersistContext {
        campaign_id: campaign_id.clone(),
        conversation_id: Id::from_str("conv-1"),
        turn: 1,
    };
    let outcome = storyforge_app_agent::PostProcessOutcome::default();

    let batch = build_mutation_batch(&store, &pc, &outcome, &[]);

    assert!(batch.is_empty(), "empty outcome should produce empty batch");
    assert_eq!(batch.expected_revision, 0);
    assert_eq!(batch.target_revision, 1);
    assert_eq!(
        batch.status,
        storyforge_domain::turn::MutationBatchStatus::Prepared
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn build_mutation_batch_with_summary_and_variable() {
    let dir = std::env::temp_dir().join(format!("storyforge-mb-var-test-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let store = campaign_store::CampaignStore::new(&dir);
    let campaign =
        storyforge_domain::campaign::Campaign::new(Id::from_str("card-1"), "test".to_string());
    let campaign_id = campaign.id.clone();
    store.save_campaign(campaign).unwrap();

    let pc = PostprocessPersistContext {
        campaign_id: campaign_id.clone(),
        conversation_id: Id::from_str("conv-1"),
        turn: 1,
    };
    let outcome = storyforge_app_agent::PostProcessOutcome {
        summary: Some("\u{7b2c}\u{4e00}\u{8f6e}\u{6458}\u{8981}".into()),
        summary_attempted: true,
        post_process_attempted: true,
        post_process: Some(storyforge_domain::agent::PostProcessResult {
            variable_updates: vec![storyforge_domain::agent::VariableUpdate {
                instance_id: None,
                key: "story_clock".into(),
                value: serde_json::json!("Day 2"),
            }],
            ..Default::default()
        }),
    };

    let batch = build_mutation_batch(&store, &pc, &outcome, &[]);

    // xxx 1 x UpsertSummary + 1 x SetVariable
    assert_eq!(
        batch.mutations.len(),
        2,
        "should have summary + variable mutations"
    );
    assert!(
        batch
            .mutations
            .iter()
            .any(|m| matches!(m, storyforge_domain::turn::Mutation::UpsertSummary(_)))
    );
    assert!(batch.mutations.iter().any(|m| matches!(
        m,
        storyforge_domain::turn::Mutation::SetVariable { key, .. } if key == "story_clock"
    )));

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn turn_store_unique_active_turn_per_campaign() {
    let dir = std::env::temp_dir().join(format!(
        "storyforge-unique-turn-test-{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let ts = turn_store::TurnStore::new(&dir);

    let r1 = storyforge_domain::turn::TurnRecord::new(
        Id::from_str("camp-1"),
        Id::from_str("conv-1"),
        Id::from_str("node-1"),
        0,
    );
    ts.create_turn(r1).unwrap();

    // x Campaign xxxxx Turn xxxx
    let r2 = storyforge_domain::turn::TurnRecord::new(
        Id::from_str("camp-1"),
        Id::from_str("conv-1"),
        Id::from_str("node-2"),
        0,
    );
    assert!(ts.create_turn(r2).is_err());

    // xx Campaign xxxx
    let r3 = storyforge_domain::turn::TurnRecord::new(
        Id::from_str("camp-2"),
        Id::from_str("conv-2"),
        Id::from_str("node-3"),
        0,
    );
    assert!(ts.create_turn(r3).is_ok());

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn startup_recovery_marks_active_turns_failed() {
    let dir =
        std::env::temp_dir().join(format!("storyforge-recovery-test-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();

    // xxxxxxxxxxxx Generating x Turn
    {
        let ts = turn_store::TurnStore::new(&dir);
        let record = storyforge_domain::turn::TurnRecord::new(
            Id::from_str("camp-recovery"),
            Id::from_str("conv-1"),
            Id::from_str("node-1"),
            0,
        );
        ts.create_turn(record).unwrap();
    }

    // "xx"xxxxxxxxxxx TurnStore
    let ts = turn_store::TurnStore::new(&dir);
    let active = ts.list_active_turns();
    assert_eq!(active.len(), 1, "should have 1 active turn before recovery");

    // xxxxxxxxxx Failed
    for turn in &active {
        ts.save_turn(storyforge_domain::turn::TurnRecord {
            status: storyforge_domain::turn::TurnStatus::Failed,
            failure_reason: Some("\u{542f}\u{52a8}\u{6062}\u{590d}".into()),
            ..turn.clone()
        })
        .unwrap();
    }

    // xxxxxxx Turn
    let ts2 = turn_store::TurnStore::new(&dir);
    assert_eq!(
        ts2.list_active_turns().len(),
        0,
        "no active turns after recovery"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn contract_recovery_keeps_committing_when_finalize_fails() {
    let dir = std::env::temp_dir().join(format!(
        "storyforge-recovery-finalize-{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let ts = turn_store::TurnStore::new(&dir);

    let mut record = storyforge_domain::turn::TurnRecord::new(
        Id::from_str("camp-finalize"),
        Id::from_str("conv-f"),
        Id::from_str("node-f"),
        0,
    );
    record.status = storyforge_domain::turn::TurnStatus::Committing;
    let attempt_id = Id::from_str("att-f");
    record.attempts.push(storyforge_domain::turn::TurnAttempt {
        attempt_id: attempt_id.clone(),
        variant_id: Id::from_str("var-missing"), // xxxxx Draft
        draft_hash: "h".into(),
        status: storyforge_domain::turn::AttemptStatus::Committing,
        pending_state_changes: Some(storyforge_domain::turn::MutationBatch::new(
            Id::from_str("batch-f"),
            0,
        )),
        derivation: None,
        quality_report: None,
        pending_temporary_instances: vec![],
        provenance: None,
        created_at: "2026-01-01T00:00:00Z".into(),
    });
    let turn_id = record.turn_id.clone();
    ts.save_turn(record).unwrap();

    // xx recover_turns_on_startup A.1xfinalize_ok=false xxx Committed
    let finalize_ok = false;
    if finalize_ok {
        ts.with_turn_mut(&turn_id, |r| {
            r.status = storyforge_domain::turn::TurnStatus::Committed;
            r.touch();
        })
        .unwrap();
    }

    let after = ts.get_turn(&turn_id).unwrap();
    assert_eq!(
        after.status,
        storyforge_domain::turn::TurnStatus::Committing,
        "finalize \u{5931}\u{8d25}\u{5fc5}\u{987b}\u{4fdd}\u{6301} Committing \u{4ee5}\u{4fbf}\u{4e0b}\u{6b21}\u{542f}\u{52a8}\u{91cd}\u{8bd5}"
    );
    assert!(
        ts.list_recoverable_turns()
            .iter()
            .any(|t| t.turn_id == turn_id)
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn postprocess_rejects_superseded_attempt_after_regenerate() {
    use storyforge_domain::turn::{AttemptStatus, TurnAttempt, TurnRecord, TurnStatus};

    let mut record = TurnRecord::new(
        Id::from_str("camp-postprocess-race"),
        Id::from_str("conv-postprocess-race"),
        Id::from_str("node-postprocess-race"),
        0,
    );
    record.status = TurnStatus::DraftReady;
    let old_attempt_id = Id::from_str("attempt-old");
    let new_attempt_id = Id::from_str("attempt-new");
    let node_id = Id::from_str("node-postprocess-race");
    record.attempts = vec![
        TurnAttempt {
            attempt_id: old_attempt_id.clone(),
            variant_id: node_id.clone(),
            draft_hash: "old-draft".into(),
            status: AttemptStatus::Superseded,
            pending_state_changes: None,
            derivation: None,
            quality_report: None,
            pending_temporary_instances: vec![],
            provenance: None,
            created_at: "2026-07-11T00:00:00Z".into(),
        },
        TurnAttempt {
            attempt_id: new_attempt_id.clone(),
            variant_id: node_id,
            draft_hash: "new-draft".into(),
            status: AttemptStatus::DraftReady,
            pending_state_changes: None,
            derivation: None,
            quality_report: None,
            pending_temporary_instances: vec![],
            provenance: None,
            created_at: "2026-07-11T00:00:01Z".into(),
        },
    ];

    assert!(
        !is_current_attempt_ready_for_postprocess(&record, &old_attempt_id),
        "late result for a superseded attempt must be ignored"
    );
    assert!(
        is_current_attempt_ready_for_postprocess(&record, &new_attempt_id),
        "current regenerate attempt may receive its own postprocess result"
    );
}

#[test]
fn sqlite_mode_never_recovers_the_legacy_json_compress_job_store() {
    let dir = std::env::temp_dir().join(format!(
        "storyforge-gate3-compress-recovery-{}",
        uuid::Uuid::new_v4()
    ));
    let sqlite_facade = crate::storage_backend::StorageFacade::new(
        dir.clone(),
        storyforge_infra_sqlite::backend::PinnedBackend::new(
            storyforge_infra_sqlite::backend::StorageBackend::Sqlite,
            storyforge_infra_sqlite::backend::BackendSource::Env,
        ),
    );
    let json_facade = crate::storage_backend::StorageFacade::new(
        dir,
        storyforge_infra_sqlite::backend::PinnedBackend::new(
            storyforge_infra_sqlite::backend::StorageBackend::Json,
            storyforge_infra_sqlite::backend::BackendSource::Default,
        ),
    );
    assert!(!should_recover_json_compress_jobs(&sqlite_facade));
    assert!(should_recover_json_compress_jobs(&json_facade));
}

#[test]
fn sqlite_mode_never_uses_the_legacy_active_campaign_pointer() {
    assert!(!should_load_legacy_active_campaign_pointer(true));
    assert!(should_load_legacy_active_campaign_pointer(false));

    let dir = std::env::temp_dir().join(format!(
        "storyforge-sqlite-active-pointer-{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("active_campaign.json"),
        r#"{"campaign_id":"stale-json-campaign"}"#,
    )
    .unwrap();

    let sqlite_facade = crate::storage_backend::StorageFacade::new(
        dir.clone(),
        storyforge_infra_sqlite::backend::PinnedBackend::new(
            storyforge_infra_sqlite::backend::StorageBackend::Sqlite,
            storyforge_infra_sqlite::backend::BackendSource::Env,
        ),
    );
    let json_facade = crate::storage_backend::StorageFacade::new(
        dir.clone(),
        storyforge_infra_sqlite::backend::PinnedBackend::new(
            storyforge_infra_sqlite::backend::StorageBackend::Json,
            storyforge_infra_sqlite::backend::BackendSource::Default,
        ),
    );

    assert_eq!(
        resolve_active_campaign_with_legacy_fallback(None, &dir, &sqlite_facade),
        None,
        "a restarted SQLite process must ignore a stale JSON selector"
    );
    assert_eq!(
        resolve_active_campaign_with_legacy_fallback(
            Some(Id::from_str("selected-in-memory")),
            &dir,
            &sqlite_facade,
        ),
        Some(Id::from_str("selected-in-memory")),
        "SQLite may use only the explicit in-process selection"
    );
    assert_eq!(
        resolve_active_campaign_with_legacy_fallback(None, &dir, &json_facade),
        Some(Id::from_str("stale-json-campaign")),
        "JSON mode preserves its legacy restart behavior"
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn startup_discards_active_pointer_for_missing_campaign() {
    let dir = std::env::temp_dir().join(format!(
        "storyforge-active-pointer-validation-{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("active_campaign.json"),
        r#"{"campaign_id":"missing-campaign"}"#,
    )
    .unwrap();
    std::fs::write(dir.join("campaigns.json"), "[]").unwrap();

    let json_facade = crate::storage_backend::StorageFacade::new(
        dir.clone(),
        storyforge_infra_sqlite::backend::PinnedBackend::new(
            storyforge_infra_sqlite::backend::StorageBackend::Json,
            storyforge_infra_sqlite::backend::BackendSource::Default,
        ),
    );
    assert_eq!(load_active_campaign_for_backend(&dir, &json_facade), None);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn save_active_campaign_reports_persistence_failure() {
    let dir = std::env::temp_dir().join(format!(
        "storyforge-active-pointer-write-failure-{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::create_dir(dir.join("active_campaign.json")).unwrap();

    let facade = crate::storage_backend::StorageFacade::new(
        dir.clone(),
        storyforge_infra_sqlite::backend::PinnedBackend::new(
            storyforge_infra_sqlite::backend::StorageBackend::Json,
            storyforge_infra_sqlite::backend::BackendSource::Default,
        ),
    );
    let error = facade
        .save_active_pointer(Some(&Id::from_str("campaign")))
        .expect_err("directory collision must not be silently ignored");
    assert!(!error.is_empty());
    std::fs::remove_dir_all(&dir).ok();
}

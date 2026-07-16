use storyforge_infra_sqlite::{Database, capture_audit_snapshot, migrate};

#[test]
fn audit_snapshot_is_consistent_counted_and_content_bound() {
    let dir = tempfile::tempdir().unwrap();
    let mut db = Database::open(dir.path().join("audit.sqlite3")).unwrap();
    migrate(&mut db).unwrap();
    let conn = db.connection_mut();
    conn.execute(
        "INSERT INTO character_cards (card_id, name, payload_json) VALUES ('card-1', 'Fixture', '{}')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO campaigns (campaign_id, card_id, name, created_at, payload_json) VALUES ('campaign-1', 'card-1', 'Fixture', 'now', '{}')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO conversations (conversation_id, campaign_id, archived_upto, created_at, updated_at, payload_json) VALUES ('conversation-1', 'campaign-1', 0, 'now', 'now', '{\"nodes\":[],\"updated_at\":\"now\"}')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO turns (turn_id, campaign_id, conversation_id, input_node_id, base_campaign_revision, status, created_at, updated_at, payload_json) VALUES ('turn-1', 'campaign-1', 'conversation-1', 'input-1', 0, 'committed', 'now', 'now', '{}')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO turn_attempts (attempt_id, turn_id, variant_id, draft_hash, status, created_at, payload_json) VALUES ('attempt-1', 'turn-1', 'variant-1', 'hash', 'committed', 'now', '{}')",
        [],
    )
    .unwrap();

    let first = capture_audit_snapshot(&mut db).unwrap();
    assert_eq!(first.sqlite_schema_version, 4);
    assert_eq!(first.turns, 1);
    assert_eq!(first.attempts, 1);
    assert_eq!(first.committed_turns, 1);
    assert_eq!(first.canonical_content_sha256.len(), 64);
    assert_eq!(first.accepted_content_sha256.len(), 64);

    db.connection_mut()
        .execute(
            "UPDATE conversations SET updated_at = 'later', payload_json = '{\"nodes\":[],\"updated_at\":\"later\"}' WHERE conversation_id = 'conversation-1'",
            [],
        )
        .unwrap();
    let after_timestamp = capture_audit_snapshot(&mut db).unwrap();
    assert_eq!(
        first.accepted_content_sha256, after_timestamp.accepted_content_sha256,
        "accepted projection must ignore conversation updated_at churn"
    );
    db.connection_mut()
        .execute(
            "INSERT INTO turns (turn_id, campaign_id, conversation_id, input_node_id, base_campaign_revision, status, created_at, updated_at, payload_json) VALUES ('turn-failed', 'campaign-1', 'conversation-1', 'input-failed', 1, 'failed', 'later', 'later', '{}')",
            [],
        )
        .unwrap();
    db.connection_mut()
        .execute(
            "INSERT INTO turn_attempts (attempt_id, turn_id, variant_id, draft_hash, status, created_at, payload_json) VALUES ('attempt-failed', 'turn-failed', 'variant-failed', 'failed-hash', 'stale', 'later', '{}')",
            [],
        )
        .unwrap();
    db.connection_mut()
        .execute(
            "INSERT INTO preaccept_outbox (outbox_id, campaign_id, conversation_id, turn_id, attempt_id, kind, draft_hash, payload_hash, status, payload_json, created_at, updated_at) VALUES ('outbox-failed', 'campaign-1', 'conversation-1', 'turn-failed', 'attempt-failed', 'postprocess', 'failed-hash', 'payload-hash', 'applied', '{}', 'later', 'later')",
            [],
        )
        .unwrap();
    let after_failed_turn = capture_audit_snapshot(&mut db).unwrap();
    assert_ne!(
        first.canonical_content_sha256, after_failed_turn.canonical_content_sha256,
        "full audit digest must see failed lifecycle rows and volatile timestamps"
    );
    assert_eq!(
        first.accepted_content_sha256, after_failed_turn.accepted_content_sha256,
        "accepted projection must ignore failed Turn, Attempt, and outbox rows"
    );

    db.connection_mut()
        .execute(
            "UPDATE character_cards SET payload_json = '{\"changed\":true}' WHERE card_id = 'card-1'",
            [],
        )
        .unwrap();
    let second = capture_audit_snapshot(&mut db).unwrap();
    assert_ne!(
        after_failed_turn.accepted_content_sha256, second.accepted_content_sha256,
        "accepted projection must bind opaque persisted payload changes"
    );
}

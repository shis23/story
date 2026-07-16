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
        "INSERT INTO conversations (conversation_id, campaign_id, archived_upto, created_at, updated_at, payload_json) VALUES ('conversation-1', 'campaign-1', 0, 'now', 'now', '{}')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO turns (turn_id, campaign_id, conversation_id, input_node_id, base_campaign_revision, status, created_at, updated_at, payload_json) VALUES ('turn-1', 'campaign-1', 'conversation-1', 'input-1', 0, 'Committed', 'now', 'now', '{}')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO turn_attempts (attempt_id, turn_id, variant_id, draft_hash, status, created_at, payload_json) VALUES ('attempt-1', 'turn-1', 'variant-1', 'hash', 'Committed', 'now', '{}')",
        [],
    )
    .unwrap();

    let first = capture_audit_snapshot(&mut db).unwrap();
    assert_eq!(first.sqlite_schema_version, 4);
    assert_eq!(first.turns, 1);
    assert_eq!(first.attempts, 1);
    assert_eq!(first.committed_turns, 1);
    assert_eq!(first.canonical_content_sha256.len(), 64);

    db.connection_mut()
        .execute(
            "UPDATE character_cards SET payload_json = '{\"changed\":true}' WHERE card_id = 'card-1'",
            [],
        )
        .unwrap();
    let second = capture_audit_snapshot(&mut db).unwrap();
    assert_ne!(
        first.canonical_content_sha256, second.canonical_content_sha256,
        "audit digest must bind opaque persisted payload changes"
    );
}

-- Pre-accept lifecycle durable outbox / recovery ledger.
-- Candidate MutationBatch still lives on turn_attempts.payload_json; this table
-- records intentional pre-accept write steps for atomic recovery and idempotent replay.

CREATE TABLE IF NOT EXISTS preaccept_outbox (
    outbox_id TEXT PRIMARY KEY NOT NULL,
    campaign_id TEXT NOT NULL,
    conversation_id TEXT NOT NULL,
    turn_id TEXT NOT NULL,
    attempt_id TEXT NOT NULL,
    kind TEXT NOT NULL,
    draft_hash TEXT NOT NULL,
    payload_hash TEXT NOT NULL,
    status TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    FOREIGN KEY (campaign_id) REFERENCES campaigns(campaign_id),
    FOREIGN KEY (conversation_id) REFERENCES conversations(conversation_id),
    FOREIGN KEY (turn_id) REFERENCES turns(turn_id),
    FOREIGN KEY (attempt_id) REFERENCES turn_attempts(attempt_id)
);

CREATE INDEX IF NOT EXISTS idx_preaccept_outbox_turn_status
    ON preaccept_outbox(turn_id, status);

CREATE INDEX IF NOT EXISTS idx_preaccept_outbox_campaign_status
    ON preaccept_outbox(campaign_id, status);

CREATE INDEX IF NOT EXISTS idx_preaccept_outbox_attempt
    ON preaccept_outbox(attempt_id, kind);

-- At most one pending outbox row per attempt+kind (idempotent replay key surface).
CREATE UNIQUE INDEX IF NOT EXISTS idx_preaccept_outbox_attempt_kind_pending
    ON preaccept_outbox(attempt_id, kind)
    WHERE status = 'pending';

CREATE INDEX IF NOT EXISTS idx_turn_attempts_status
    ON turn_attempts(status);

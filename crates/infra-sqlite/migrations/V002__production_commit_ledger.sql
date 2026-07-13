-- Persistent idempotency ledger for production-style SQLite Turn accepts.

CREATE TABLE IF NOT EXISTS mutation_commits (
    commit_id TEXT PRIMARY KEY NOT NULL,
    campaign_id TEXT NOT NULL,
    turn_id TEXT NOT NULL,
    attempt_id TEXT NOT NULL,
    expected_revision INTEGER NOT NULL,
    target_revision INTEGER NOT NULL,
    terminal_status TEXT NOT NULL,
    payload_hash TEXT NOT NULL,
    committed_at TEXT NOT NULL,
    FOREIGN KEY (campaign_id) REFERENCES campaigns(campaign_id),
    FOREIGN KEY (turn_id) REFERENCES turns(turn_id),
    FOREIGN KEY (attempt_id) REFERENCES turn_attempts(attempt_id)
);

CREATE INDEX IF NOT EXISTS idx_mutation_commits_campaign
    ON mutation_commits(campaign_id, target_revision);

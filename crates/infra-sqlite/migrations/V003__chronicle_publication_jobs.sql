-- Durable Chronicle B/C publication job / replay ledger.

CREATE TABLE IF NOT EXISTS chronicle_publication_jobs (
    publication_id TEXT PRIMARY KEY NOT NULL,
    campaign_id TEXT NOT NULL,
    job_id TEXT,
    base_chronicle_revision INTEGER NOT NULL,
    target_chronicle_revision INTEGER NOT NULL,
    parent_ids_json TEXT NOT NULL,
    child_covered_by_json TEXT NOT NULL,
    payload_hash TEXT NOT NULL,
    status TEXT NOT NULL,
    created_at TEXT NOT NULL,
    completed_at TEXT,
    FOREIGN KEY (campaign_id) REFERENCES campaigns(campaign_id)
);

CREATE INDEX IF NOT EXISTS idx_chronicle_publication_jobs_campaign
    ON chronicle_publication_jobs(campaign_id, completed_at);

CREATE INDEX IF NOT EXISTS idx_chronicle_publication_jobs_job_id
    ON chronicle_publication_jobs(job_id);

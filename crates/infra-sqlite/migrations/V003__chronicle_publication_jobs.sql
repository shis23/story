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

CREATE UNIQUE INDEX IF NOT EXISTS idx_chronicle_publication_jobs_job_id
    ON chronicle_publication_jobs(job_id)
    WHERE job_id IS NOT NULL AND job_id <> '';

-- Importer re-checks after BEGIN IMMEDIATE; this is the durable last line of defence.
CREATE UNIQUE INDEX IF NOT EXISTS idx_import_runs_completed_manifest
    ON import_runs(source_manifest_hash)
    WHERE status = 'completed';

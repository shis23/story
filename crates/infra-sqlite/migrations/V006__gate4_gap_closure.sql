-- Gate 4: SQLite-native gap closure (Meta/MVU/Chronicle/WorldInfo support tables).
-- compress_jobs 镜像 tauri-app CompressJobStore 的 compress_jobs.json 语义
-- （campaign 级 open 去重、attempts 重试上限、启动 Running→Pending 恢复）。

CREATE TABLE IF NOT EXISTS chronicle_compress_jobs (
    job_id TEXT PRIMARY KEY NOT NULL,
    campaign_id TEXT NOT NULL,
    conversation_id TEXT,
    lineage_id TEXT,
    kind TEXT NOT NULL DEFAULT 'auto',
    status TEXT NOT NULL,
    attempts INTEGER NOT NULL DEFAULT 0,
    max_attempts INTEGER NOT NULL DEFAULT 5,
    last_error TEXT,
    uncovered_a_at_enqueue INTEGER NOT NULL DEFAULT 0,
    uncovered_b_at_enqueue INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    FOREIGN KEY (campaign_id) REFERENCES campaigns(campaign_id)
);

CREATE INDEX IF NOT EXISTS idx_compress_jobs_campaign_status
    ON chronicle_compress_jobs(campaign_id, status);

-- 每个 campaign 至多一个 open（pending/running）job：enqueue 幂等去重面。
CREATE UNIQUE INDEX IF NOT EXISTS idx_compress_jobs_open_per_campaign
    ON chronicle_compress_jobs(campaign_id)
    WHERE status IN ('pending', 'running');

-- 本局世界书（JSON 布局 campaign_world_info/{campaign_id}.json 的 SQLite 权威）。
-- payload_json = WorldInfoBook 全量 JSON。
CREATE TABLE IF NOT EXISTS campaign_world_info (
    campaign_id TEXT PRIMARY KEY NOT NULL,
    payload_json TEXT NOT NULL,
    updated_at TEXT NOT NULL DEFAULT '',
    FOREIGN KEY (campaign_id) REFERENCES campaigns(campaign_id)
);

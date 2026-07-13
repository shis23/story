-- StoryForge SQLite schema v1
-- 主键使用稳定 Id 字符串；显示 code 不作为主键。
-- 扩展/未稳定字段进入 payload_json。

CREATE TABLE IF NOT EXISTS character_cards (
    card_id TEXT PRIMARY KEY NOT NULL,
    source_character_id TEXT,
    name TEXT NOT NULL,
    imported_at TEXT,
    payload_json TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS campaigns (
    campaign_id TEXT PRIMARY KEY NOT NULL,
    card_id TEXT NOT NULL,
    name TEXT NOT NULL,
    conversation_id TEXT,
    revision INTEGER NOT NULL DEFAULT 0,
    chronicle_revision INTEGER NOT NULL DEFAULT 0,
    lineage_id TEXT,
    story_clock TEXT NOT NULL DEFAULT 'Day 1',
    created_at TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    FOREIGN KEY (card_id) REFERENCES character_cards(card_id)
);

CREATE INDEX IF NOT EXISTS idx_campaigns_card_id ON campaigns(card_id);

CREATE TABLE IF NOT EXISTS character_instances (
    instance_id TEXT PRIMARY KEY NOT NULL,
    campaign_id TEXT NOT NULL,
    definition_id TEXT,
    name TEXT NOT NULL,
    is_temporary INTEGER NOT NULL DEFAULT 0,
    payload_json TEXT NOT NULL,
    FOREIGN KEY (campaign_id) REFERENCES campaigns(campaign_id)
);

CREATE INDEX IF NOT EXISTS idx_instances_campaign ON character_instances(campaign_id);

CREATE TABLE IF NOT EXISTS character_knowledge (
    knowledge_id TEXT PRIMARY KEY NOT NULL,
    campaign_id TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    FOREIGN KEY (campaign_id) REFERENCES campaigns(campaign_id)
);

CREATE INDEX IF NOT EXISTS idx_knowledge_campaign ON character_knowledge(campaign_id);

CREATE TABLE IF NOT EXISTS story_tasks (
    task_id TEXT PRIMARY KEY NOT NULL,
    campaign_id TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    FOREIGN KEY (campaign_id) REFERENCES campaigns(campaign_id)
);

CREATE INDEX IF NOT EXISTS idx_tasks_campaign ON story_tasks(campaign_id);

CREATE TABLE IF NOT EXISTS conversations (
    conversation_id TEXT PRIMARY KEY NOT NULL,
    campaign_id TEXT,
    character_id TEXT,
    archived_upto INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    payload_json TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_conversations_campaign ON conversations(campaign_id);

CREATE TABLE IF NOT EXISTS turns (
    turn_id TEXT PRIMARY KEY NOT NULL,
    campaign_id TEXT NOT NULL,
    conversation_id TEXT NOT NULL,
    input_node_id TEXT NOT NULL,
    base_campaign_revision INTEGER NOT NULL,
    status TEXT NOT NULL,
    accepted_attempt_id TEXT,
    failure_reason TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    FOREIGN KEY (campaign_id) REFERENCES campaigns(campaign_id),
    FOREIGN KEY (conversation_id) REFERENCES conversations(conversation_id)
);

CREATE INDEX IF NOT EXISTS idx_turns_campaign_status ON turns(campaign_id, status);

CREATE TABLE IF NOT EXISTS turn_attempts (
    attempt_id TEXT PRIMARY KEY NOT NULL,
    turn_id TEXT NOT NULL,
    variant_id TEXT NOT NULL,
    draft_hash TEXT NOT NULL,
    status TEXT NOT NULL,
    created_at TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    FOREIGN KEY (turn_id) REFERENCES turns(turn_id)
);

CREATE INDEX IF NOT EXISTS idx_attempts_turn ON turn_attempts(turn_id);
CREATE INDEX IF NOT EXISTS idx_attempts_variant ON turn_attempts(variant_id);

CREATE TABLE IF NOT EXISTS round_summaries (
    summary_id TEXT PRIMARY KEY NOT NULL,
    campaign_id TEXT NOT NULL,
    conversation_id TEXT NOT NULL,
    lineage_id TEXT,
    level INTEGER NOT NULL DEFAULT 0,
    turn INTEGER NOT NULL,
    turn_end INTEGER NOT NULL DEFAULT 0,
    code TEXT,
    headline TEXT,
    covered_by TEXT,
    content TEXT NOT NULL,
    created_at TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    FOREIGN KEY (campaign_id) REFERENCES campaigns(campaign_id),
    FOREIGN KEY (conversation_id) REFERENCES conversations(conversation_id),
    FOREIGN KEY (covered_by) REFERENCES round_summaries(summary_id)
);

CREATE INDEX IF NOT EXISTS idx_summaries_campaign_lineage
    ON round_summaries(campaign_id, lineage_id);
CREATE INDEX IF NOT EXISTS idx_summaries_level ON round_summaries(campaign_id, level);

CREATE TABLE IF NOT EXISTS round_summary_covers (
    parent_id TEXT NOT NULL,
    child_id TEXT NOT NULL,
    PRIMARY KEY (parent_id, child_id),
    FOREIGN KEY (parent_id) REFERENCES round_summaries(summary_id),
    FOREIGN KEY (child_id) REFERENCES round_summaries(summary_id)
);

CREATE TABLE IF NOT EXISTS import_runs (
    run_id TEXT PRIMARY KEY NOT NULL,
    source_root TEXT NOT NULL,
    source_manifest_hash TEXT NOT NULL,
    status TEXT NOT NULL,
    started_at TEXT NOT NULL,
    finished_at TEXT,
    error TEXT
);

CREATE INDEX IF NOT EXISTS idx_import_runs_hash_status
    ON import_runs(source_manifest_hash, status);

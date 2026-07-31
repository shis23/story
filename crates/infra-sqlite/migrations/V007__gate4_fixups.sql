-- Gate 4 review fixups (2026-07-31):
--   1) A single Chronicle compress job may produce multiple publications
--      (A→B then B→C). Uniqueness must be per (job_id, batch_index), not per
--      job_id, or the second batch is misread as a duplicate/late result.
--   2) SQLite-native character library mirroring the JSON CharacterStore
--      (`characters.json`) semantics so CharacterCommands work in SQLite mode.

ALTER TABLE chronicle_publication_jobs ADD COLUMN batch_index INTEGER NOT NULL DEFAULT 0;

DROP INDEX IF EXISTS idx_chronicle_publication_jobs_job_id;
CREATE UNIQUE INDEX IF NOT EXISTS idx_chronicle_publication_jobs_job_batch
    ON chronicle_publication_jobs(job_id, batch_index)
    WHERE job_id IS NOT NULL AND job_id <> '';

CREATE TABLE IF NOT EXISTS characters (
    character_id TEXT PRIMARY KEY NOT NULL,
    source_character_id TEXT,
    name TEXT NOT NULL,
    info_json TEXT NOT NULL,
    imported_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_characters_name ON characters(name);

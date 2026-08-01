-- Gate 5 authority binding: bind a cutover identity into the database so a
-- marker can only authorize the exact DB it was published with.
--
-- Additive only: existing rows keep working; new cutovers write the binding.

CREATE TABLE IF NOT EXISTS authority_binding (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    authority_id TEXT NOT NULL,
    cutover_nonce TEXT NOT NULL,
    created_at TEXT NOT NULL
);

ALTER TABLE import_runs ADD COLUMN authority_id TEXT;
ALTER TABLE import_runs ADD COLUMN cutover_nonce TEXT;

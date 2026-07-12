use std::path::PathBuf;

/// SQLite 基础层错误。
#[derive(Debug, thiserror::Error)]
pub enum SqliteError {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("json: {0}")]
    Json(#[from] serde_json::Error),

    #[error("migration checksum mismatch for version {version}: expected {expected}, got {actual}")]
    MigrationChecksumMismatch {
        version: i64,
        expected: String,
        actual: String,
    },

    #[error("migration failed at version {version} ({name}): {message}")]
    MigrationFailed {
        version: i64,
        name: String,
        message: String,
    },

    #[error("import source not found: {0}")]
    ImportSourceMissing(PathBuf),

    #[error("import rejected corrupt input: {0}")]
    CorruptImportInput(String),

    #[error("import already completed with different payload for id {0}")]
    ImportConflict(String),

    #[error("unit of work already finished")]
    UnitOfWorkFinished,

    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, SqliteError>;

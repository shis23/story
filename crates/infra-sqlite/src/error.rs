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

    #[error("invalid migration set: {0}")]
    InvalidMigrationSet(String),

    #[error("import source not found: {0}")]
    ImportSourceMissing(PathBuf),

    #[error("import rejected corrupt input: {0}")]
    CorruptImportInput(String),

    #[error("import already completed with different payload for id {0}")]
    ImportConflict(String),

    #[error("unit of work already finished")]
    UnitOfWorkFinished,

    #[error("production repository conflict: {0}")]
    Conflict(String),

    /// Accept 的 revision CAS 失败（V7：typed 跨边界，适配层不得再按子串分类——
    /// integrity 分歧等错误消息同样含 "revision" 字样，子串匹配会把 DB 损坏
    /// 误报成"回合过期"）。
    #[error(
        "revision conflict: campaign={campaign}, turn_base={turn_base}, expected={expected}, target={target}"
    )]
    RevisionConflict {
        campaign: u64,
        turn_base: u64,
        expected: u64,
        target: u64,
    },

    #[error("production repository record not found: {0}")]
    RecordNotFound(String),

    /// 领域层 NotFound 的镜像（JSON 路径经 TauriCommandError::not_found 渲染
    /// "Not found: {0}"）；等价矩阵要求同一领域错误双后端文本一致，不能把
    /// 领域校验错误包装成存储层 RecordNotFound。
    #[error("Not found: {0}")]
    NotFound(String),

    /// 领域层校验错误的镜像（JSON 路径渲染 "Validation error: {0}"）。
    #[error("Validation error: {0}")]
    Validation(String),

    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, SqliteError>;

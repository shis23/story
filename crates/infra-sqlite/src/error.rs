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

    /// S-01（默认后端 P0）：父集合源文件缺失/为空而子集合非空——无法区分
    /// 「源数据不完整」与「真实为空」。此时按「缺失 = 空集合」继续会把全部
    /// 子行判成孤儿后发布空库并写入权威 marker（用户数据静默消失），因此必须
    /// fail-closed 并把原因明确告知操作者。
    #[error("import source incomplete: {0}")]
    ImportSourceIncomplete(String),

    /// N-R1-04：`Path::exists()` 把「读不了」吞成「不存在」（stat 的权限/IO
    /// 错误一律返回 false）。optional 读法随后把整个集合当空集合导入，等于
    /// 静默空库。这里把「stat 失败但不是 NotFound」报成独立错误：既明确区分
    /// 「文件没了」与「文件在但读不了」，又保证 fail-closed。
    #[error("import source unreadable: {0}")]
    ImportSourceUnreadable(String),

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

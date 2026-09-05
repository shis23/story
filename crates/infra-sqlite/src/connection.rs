use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags};

use crate::error::{Result, SqliteError};

/// StoryForge 数据库 `PRAGMA application_id` 魔数（ASCII "STFG"）。
///
/// 只读所有权探测与 cutover 发布路径都依赖该常量，避免把外部 SQLite 文件
/// 误判为自身产物。写入是幂等的：已是该值时不再改写。
pub const STORYFORGE_APPLICATION_ID: i32 = 0x5354_4647; // 'S''T''F''G'

/// 打开并配置 StoryForge SQLite 连接。
///
/// 固定 PRAGMA（见 ADR 0001）：
/// - journal_mode=WAL
/// - foreign_keys=ON
/// - busy_timeout=5000
/// - synchronous=NORMAL
/// - temp_store=MEMORY
/// - application_id=STORYFORGE_APPLICATION_ID（幂等）
pub struct Database {
    path: PathBuf,
    conn: Connection,
}

impl Database {
    /// 在 `path` 打开（或创建）数据库文件并应用 PRAGMA。
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let conn = Connection::open_with_flags(
            &path,
            OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_CREATE
                | OpenFlags::SQLITE_OPEN_FULL_MUTEX,
        )?;
        configure_connection(&conn)?;

        Ok(Self { path, conn })
    }

    /// 内存库（测试用）。
    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        configure_connection(&conn)?;
        Ok(Self {
            path: PathBuf::from(":memory:"),
            conn,
        })
    }

    /// 只读打开：绝不应用任何写 PRAGMA（WAL / application_id）。
    ///
    /// 用于启动期对 marker 所指 DB 的只读探测（Gate 8 审查 P2-A1）：检查外部
    /// 或他进程占用的文件时不得产生任何写副作用，也不能把外部库烙上
    /// StoryForge 标记或改写其 journal 模式。
    pub fn open_readonly(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        let conn = Connection::open_with_flags(
            &path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        Ok(Self { path, conn })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn connection(&self) -> &Connection {
        &self.conn
    }

    pub fn connection_mut(&mut self) -> &mut Connection {
        &mut self.conn
    }

    /// 只读查询 schema_migrations 是否存在。
    pub fn has_schema_migrations_table(&self) -> Result<bool> {
        let mut stmt = self.conn.prepare(
            "SELECT 1 FROM sqlite_master WHERE type='table' AND name='schema_migrations' LIMIT 1",
        )?;
        let exists = stmt.exists([])?;
        Ok(exists)
    }
}

fn configure_connection(conn: &Connection) -> Result<()> {
    conn.busy_timeout(std::time::Duration::from_secs(5))?;
    // Concurrent first opens can deadlock while upgrading the journal lock.
    // SQLite may return BUSY without calling its busy handler in that case.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        match conn.pragma_update(None, "journal_mode", "WAL") {
            Err(rusqlite::Error::SqliteFailure(error, _))
                if error.code == rusqlite::ErrorCode::DatabaseBusy
                    && std::time::Instant::now() < deadline =>
            {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            result => {
                result?;
                break;
            }
        }
    }
    conn.pragma_update(None, "foreign_keys", "ON")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.pragma_update(None, "temp_store", "MEMORY")?;
    ensure_application_id(conn)?;

    // 校验 foreign_keys 确实打开（部分连接模式可能忽略）
    let fk: i64 = conn.query_row("PRAGMA foreign_keys", [], |row| row.get(0))?;
    if fk != 1 {
        return Err(SqliteError::Other(
            "failed to enable foreign_keys PRAGMA".into(),
        ));
    }
    Ok(())
}

/// 幂等写入 application_id；已是目标值时不改写。
fn ensure_application_id(conn: &Connection) -> Result<()> {
    let current: i64 = conn.query_row("PRAGMA application_id", [], |row| row.get(0))?;
    if current != i64::from(STORYFORGE_APPLICATION_ID) {
        conn.pragma_update(None, "application_id", STORYFORGE_APPLICATION_ID)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn open_applies_required_pragmas() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("storyforge.sqlite3");
        let db = Database::open(&path).unwrap();

        let journal: String = db
            .connection()
            .query_row("PRAGMA journal_mode", [], |r| r.get(0))
            .unwrap();
        let fk: i64 = db
            .connection()
            .query_row("PRAGMA foreign_keys", [], |r| r.get(0))
            .unwrap();
        let sync: i64 = db
            .connection()
            .query_row("PRAGMA synchronous", [], |r| r.get(0))
            .unwrap();

        assert_eq!(journal.to_lowercase(), "wal");
        assert_eq!(fk, 1);
        // NORMAL == 1
        assert_eq!(sync, 1);
        assert!(path.exists());
    }

    #[test]
    fn open_creates_parent_directories() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("nested").join("a").join("db.sqlite3");
        let db = Database::open(&path).unwrap();
        assert_eq!(db.path(), path.as_path());
        assert!(path.exists());
    }

    #[test]
    fn concurrent_first_open_preserves_wal_and_application_identity() {
        use std::sync::{Arc, Barrier};
        for _ in 0..12 {
            let dir = TempDir::new().unwrap();
            let path = dir.path().join("racing.sqlite3");
            let barrier = Arc::new(Barrier::new(6));
            let workers: Vec<_> = (0..6)
                .map(|_| {
                    let path = path.clone();
                    let barrier = barrier.clone();
                    std::thread::spawn(move || {
                        barrier.wait();
                        Database::open(path)
                    })
                })
                .collect();
            for worker in workers {
                let db = worker.join().unwrap().unwrap();
                let journal: String = db
                    .connection()
                    .query_row("PRAGMA journal_mode", [], |row| row.get(0))
                    .unwrap();
                let identity: i32 = db
                    .connection()
                    .query_row("PRAGMA application_id", [], |row| row.get(0))
                    .unwrap();
                assert_eq!(journal, "wal");
                assert_eq!(identity, STORYFORGE_APPLICATION_ID);
            }
        }
    }
}

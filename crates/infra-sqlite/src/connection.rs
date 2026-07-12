use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags};

use crate::error::{Result, SqliteError};

/// 打开并配置 StoryForge SQLite 连接。
///
/// 固定 PRAGMA（见 ADR 0001）：
/// - journal_mode=WAL
/// - foreign_keys=ON
/// - busy_timeout=5000
/// - synchronous=NORMAL
/// - temp_store=MEMORY
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
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    conn.pragma_update(None, "busy_timeout", 5000i64)?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.pragma_update(None, "temp_store", "MEMORY")?;

    // 校验 foreign_keys 确实打开（部分连接模式可能忽略）
    let fk: i64 = conn.query_row("PRAGMA foreign_keys", [], |row| row.get(0))?;
    if fk != 1 {
        return Err(SqliteError::Other(
            "failed to enable foreign_keys PRAGMA".into(),
        ));
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
}

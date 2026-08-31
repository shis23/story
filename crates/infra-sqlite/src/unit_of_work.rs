use rusqlite::{Connection, TransactionBehavior};

use crate::error::{Result, SqliteError};

/// 事务封装雏形。
///
/// Drop 时若尚未 commit，则 rollback（rusqlite Transaction 默认行为）。
pub struct UnitOfWork<'conn> {
    tx: Option<rusqlite::Transaction<'conn>>,
}

impl<'conn> UnitOfWork<'conn> {
    pub fn begin(conn: &'conn mut Connection) -> Result<Self> {
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        Ok(Self { tx: Some(tx) })
    }

    pub fn transaction(&self) -> Result<&rusqlite::Transaction<'conn>> {
        self.tx.as_ref().ok_or(SqliteError::UnitOfWorkFinished)
    }

    pub fn transaction_mut(&mut self) -> Result<&mut rusqlite::Transaction<'conn>> {
        self.tx.as_mut().ok_or(SqliteError::UnitOfWorkFinished)
    }

    pub fn execute(&self, sql: &str, params: impl rusqlite::Params) -> Result<usize> {
        let n = self.transaction()?.execute(sql, params)?;
        Ok(n)
    }

    pub fn commit(mut self) -> Result<()> {
        let tx = self.tx.take().ok_or(SqliteError::UnitOfWorkFinished)?;
        tx.commit()?;
        Ok(())
    }

    pub fn rollback(mut self) -> Result<()> {
        let tx = self.tx.take().ok_or(SqliteError::UnitOfWorkFinished)?;
        tx.rollback()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connection::Database;

    fn setup() -> Database {
        let db = Database::open_in_memory().unwrap();
        db.connection()
            .execute_batch("CREATE TABLE t (id INTEGER PRIMARY KEY, v TEXT NOT NULL);")
            .unwrap();
        db
    }

    #[test]
    fn commit_persists_writes() {
        let mut db = setup();
        {
            let uow = UnitOfWork::begin(db.connection_mut()).unwrap();
            uow.execute("INSERT INTO t (id, v) VALUES (1, 'a')", [])
                .unwrap();
            uow.commit().unwrap();
        }
        let count: i64 = db
            .connection()
            .query_row("SELECT COUNT(*) FROM t", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn rollback_discards_writes() {
        let mut db = setup();
        {
            let uow = UnitOfWork::begin(db.connection_mut()).unwrap();
            uow.execute("INSERT INTO t (id, v) VALUES (1, 'a')", [])
                .unwrap();
            uow.rollback().unwrap();
        }
        let count: i64 = db
            .connection()
            .query_row("SELECT COUNT(*) FROM t", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn drop_without_commit_rolls_back() {
        let mut db = setup();
        {
            let uow = UnitOfWork::begin(db.connection_mut()).unwrap();
            uow.execute("INSERT INTO t (id, v) VALUES (1, 'a')", [])
                .unwrap();
            // drop without commit
        }
        let count: i64 = db
            .connection()
            .query_row("SELECT COUNT(*) FROM t", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }
}

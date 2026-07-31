use sha2::{Digest, Sha256};

use crate::connection::Database;
use crate::error::{Result, SqliteError};
use crate::unit_of_work::UnitOfWork;

/// 一条编号 migration。
#[derive(Debug, Clone)]
pub struct Migration {
    pub version: i64,
    pub name: &'static str,
    pub sql: &'static str,
}

impl Migration {
    pub fn checksum(&self) -> String {
        let mut hasher = Sha256::new();
        hasher.update(self.version.to_string().as_bytes());
        hasher.update(b"\0");
        hasher.update(self.name.as_bytes());
        hasher.update(b"\0");
        hasher.update(self.sql.as_bytes());
        hex_encode(hasher.finalize())
    }
}

/// 内置 migration 列表（单向、编号）。
pub fn builtin_migrations() -> Vec<Migration> {
    vec![
        Migration {
            version: 1,
            name: "init_schema_v1",
            sql: include_str!("../migrations/V001__init_schema.sql"),
        },
        Migration {
            version: 2,
            name: "production_commit_ledger",
            sql: include_str!("../migrations/V002__production_commit_ledger.sql"),
        },
        Migration {
            version: 3,
            name: "chronicle_publication_jobs",
            sql: include_str!("../migrations/V003__chronicle_publication_jobs.sql"),
        },
        Migration {
            version: 4,
            name: "preaccept_lifecycle",
            sql: include_str!("../migrations/V004__preaccept_lifecycle.sql"),
        },
        Migration {
            version: 5,
            name: "mvu_translations",
            sql: include_str!("../migrations/V005__mvu_translations.sql"),
        },
        Migration {
            version: 6,
            name: "gate4_gap_closure",
            sql: include_str!("../migrations/V006__gate4_gap_closure.sql"),
        },
        Migration {
            version: 7,
            name: "gate4_review_fixups",
            sql: include_str!("../migrations/V007__gate4_fixups.sql"),
        },
    ]
}

/// 应用全部未执行 migration；已应用的校验 checksum。
pub fn migrate(db: &mut Database) -> Result<Vec<i64>> {
    migrate_with(db, &builtin_migrations())
}

/// 可注入 migration 列表（测试用，可模拟中途失败）。
///
/// 调用方传入顺序不保证正确：本函数始终按 `version` 升序执行副本。
/// 拒绝非正 version、重复 version。
pub fn migrate_with(db: &mut Database, migrations: &[Migration]) -> Result<Vec<i64>> {
    ensure_migrations_table(db)?;

    let ordered = order_and_validate_migrations(migrations)?;

    let mut applied_now = Vec::new();
    for migration in &ordered {
        if let Some(existing) = load_applied(db, migration.version)? {
            let expected = migration.checksum();
            if existing != expected {
                return Err(SqliteError::MigrationChecksumMismatch {
                    version: migration.version,
                    expected: existing,
                    actual: expected,
                });
            }
            continue;
        }

        let applied = apply_one(db, migration).map_err(|e| match e {
            SqliteError::Sqlite(err) => SqliteError::MigrationFailed {
                version: migration.version,
                name: migration.name.to_string(),
                message: err.to_string(),
            },
            other => other,
        })?;
        if applied {
            applied_now.push(migration.version);
        }
    }
    Ok(applied_now)
}

/// 校验并按 version 升序返回 migration 副本。
fn order_and_validate_migrations(migrations: &[Migration]) -> Result<Vec<Migration>> {
    let mut ordered = migrations.to_vec();
    ordered.sort_by_key(|m| m.version);

    let mut seen = std::collections::BTreeSet::new();
    for m in &ordered {
        if m.version <= 0 {
            return Err(SqliteError::InvalidMigrationSet(format!(
                "migration version must be positive, got {}",
                m.version
            )));
        }
        if !seen.insert(m.version) {
            return Err(SqliteError::InvalidMigrationSet(format!(
                "duplicate migration version {}",
                m.version
            )));
        }
    }
    Ok(ordered)
}

fn ensure_migrations_table(db: &mut Database) -> Result<()> {
    db.connection().execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS schema_migrations (
            version INTEGER PRIMARY KEY,
            name TEXT NOT NULL,
            applied_at TEXT NOT NULL,
            checksum TEXT NOT NULL
        );
        "#,
    )?;
    Ok(())
}

fn load_applied(db: &Database, version: i64) -> Result<Option<String>> {
    let mut stmt = db
        .connection()
        .prepare("SELECT checksum FROM schema_migrations WHERE version = ?1")?;
    let mut rows = stmt.query(rusqlite::params![version])?;
    if let Some(row) = rows.next()? {
        let checksum: String = row.get(0)?;
        Ok(Some(checksum))
    } else {
        Ok(None)
    }
}

fn apply_one(db: &mut Database, migration: &Migration) -> Result<bool> {
    let checksum = migration.checksum();
    let applied_at = chrono::Utc::now().to_rfc3339();

    let uow = UnitOfWork::begin(db.connection_mut())?;
    {
        let tx = uow.transaction()?;
        if let Some(existing) = load_applied_tx(tx, migration.version)? {
            if existing != checksum {
                return Err(SqliteError::MigrationChecksumMismatch {
                    version: migration.version,
                    expected: existing,
                    actual: checksum,
                });
            }
            uow.commit()?;
            return Ok(false);
        }
        tx.execute_batch(migration.sql)?;
        tx.execute(
            r#"
            INSERT INTO schema_migrations (version, name, applied_at, checksum)
            VALUES (?1, ?2, ?3, ?4)
            "#,
            rusqlite::params![migration.version, migration.name, applied_at, checksum],
        )?;
    }
    uow.commit()?;
    Ok(true)
}

fn load_applied_tx(tx: &rusqlite::Transaction<'_>, version: i64) -> Result<Option<String>> {
    let mut stmt = tx.prepare("SELECT checksum FROM schema_migrations WHERE version = ?1")?;
    let mut rows = stmt.query(rusqlite::params![version])?;
    if let Some(row) = rows.next()? {
        Ok(Some(row.get(0)?))
    } else {
        Ok(None)
    }
}

/// 当前已应用的最大 version；无则 0。
pub fn current_version(db: &Database) -> Result<i64> {
    if !db.has_schema_migrations_table()? {
        return Ok(0);
    }
    let version: i64 = db.connection().query_row(
        "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
        [],
        |row| row.get(0),
    )?;
    Ok(version)
}

fn hex_encode(bytes: impl AsRef<[u8]>) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let bytes = bytes.as_ref();
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0xf) as usize] as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connection::Database;
    use std::sync::{Arc, Barrier};
    use tempfile::TempDir;

    #[test]
    fn migrate_applies_v1_and_is_idempotent() {
        let mut db = Database::open_in_memory().unwrap();
        let first = migrate(&mut db).unwrap();
        assert_eq!(first, vec![1, 2, 3, 4, 5, 6, 7]);
        assert_eq!(current_version(&db).unwrap(), 7);

        let second = migrate(&mut db).unwrap();
        assert!(second.is_empty());
        assert_eq!(current_version(&db).unwrap(), 7);

        // 核心表应存在
        for table in [
            "schema_migrations",
            "campaigns",
            "conversations",
            "turns",
            "turn_attempts",
            "round_summaries",
            "round_summary_covers",
            "mutation_commits",
            "chronicle_publication_jobs",
            "preaccept_outbox",
            "import_runs",
            "mvu_translations",
            "chronicle_compress_jobs",
            "campaign_world_info",
            "characters",
        ] {
            let exists: i64 = db
                .connection()
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
                    [table],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(exists, 1, "missing table {table}");
        }
    }

    #[test]
    fn existing_v1_database_upgrades_to_v2_without_losing_data() {
        let mut db = Database::open_in_memory().unwrap();
        let v1 = builtin_migrations().remove(0);
        assert_eq!(migrate_with(&mut db, &[v1]).unwrap(), vec![1]);
        db.connection()
            .execute(
                "INSERT INTO character_cards (card_id, name, payload_json) VALUES ('card-upgrade', 'Upgrade', '{}')",
                [],
            )
            .unwrap();

        assert_eq!(migrate(&mut db).unwrap(), vec![2, 3, 4, 5, 6, 7]);
        assert_eq!(current_version(&db).unwrap(), 7);
        let cards: i64 = db
            .connection()
            .query_row(
                "SELECT COUNT(*) FROM character_cards WHERE card_id = 'card-upgrade'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(cards, 1);
        for table in [
            "mutation_commits",
            "chronicle_publication_jobs",
            "preaccept_outbox",
        ] {
            let exists: i64 = db
                .connection()
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
                    [table],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(exists, 1, "missing {table}");
        }
    }

    #[test]
    fn failed_migration_does_not_leave_half_schema() {
        let mut db = Database::open_in_memory().unwrap();
        // 先建立 migrations 表
        ensure_migrations_table(&mut db).unwrap();

        let bad = Migration {
            version: 99,
            name: "broken",
            sql: "CREATE TABLE ok_part (id INTEGER PRIMARY KEY); THIS IS NOT VALID SQL;",
        };
        let err = migrate_with(&mut db, &[bad]).unwrap_err();
        match err {
            SqliteError::MigrationFailed { version, .. } => assert_eq!(version, 99),
            other => panic!("unexpected error: {other}"),
        }

        // 事务回滚：ok_part 不应存在，schema_migrations 无 99
        let table_count: i64 = db
            .connection()
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='ok_part'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(table_count, 0);

        let row_count: i64 = db
            .connection()
            .query_row(
                "SELECT COUNT(*) FROM schema_migrations WHERE version = 99",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(row_count, 0);
    }

    #[test]
    fn checksum_mismatch_is_detected() {
        let mut db = Database::open_in_memory().unwrap();
        let m1 = Migration {
            version: 1,
            name: "a",
            sql: "CREATE TABLE t1 (id INTEGER PRIMARY KEY);",
        };
        migrate_with(&mut db, std::slice::from_ref(&m1)).unwrap();

        let m1_changed = Migration {
            version: 1,
            name: "a",
            sql: "CREATE TABLE t1 (id INTEGER PRIMARY KEY, x TEXT);",
        };
        let err = migrate_with(&mut db, &[m1_changed]).unwrap_err();
        assert!(matches!(
            err,
            SqliteError::MigrationChecksumMismatch { version: 1, .. }
        ));
    }

    #[test]
    fn foreign_keys_enforced_after_v1() {
        let mut db = Database::open_in_memory().unwrap();
        migrate(&mut db).unwrap();

        let err = db.connection().execute(
            "INSERT INTO turns (turn_id, campaign_id, conversation_id, input_node_id, base_campaign_revision, status, accepted_attempt_id, failure_reason, created_at, updated_at, payload_json)
             VALUES ('t1', 'missing-campaign', 'c1', 'n1', 0, 'generating', NULL, NULL, 'now', 'now', '{}')",
            [],
        );
        assert!(err.is_err(), "FK should reject missing campaign");
    }

    #[test]
    fn migrate_with_sorts_by_version_regardless_of_input_order() {
        let mut db = Database::open_in_memory().unwrap();
        ensure_migrations_table(&mut db).unwrap();

        // Intentionally out of order: V2 before V1. Runner must sort and apply V1 first.
        let v2 = Migration {
            version: 2,
            name: "second",
            sql: "CREATE TABLE t2 (id INTEGER PRIMARY KEY);",
        };
        let v1 = Migration {
            version: 1,
            name: "first",
            sql: "CREATE TABLE t1 (id INTEGER PRIMARY KEY);",
        };
        let applied = migrate_with(&mut db, &[v2, v1]).unwrap();
        assert_eq!(applied, vec![1, 2]);
        assert_eq!(current_version(&db).unwrap(), 2);

        // Both tables exist; ordering was correct even though V2 was listed first.
        for table in ["t1", "t2"] {
            let exists: i64 = db
                .connection()
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
                    [table],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(exists, 1, "missing table {table}");
        }
    }

    #[test]
    fn migrate_with_rejects_duplicate_and_non_positive_versions() {
        let mut db = Database::open_in_memory().unwrap();
        ensure_migrations_table(&mut db).unwrap();

        let dup = [
            Migration {
                version: 1,
                name: "a",
                sql: "CREATE TABLE a (id INTEGER PRIMARY KEY);",
            },
            Migration {
                version: 1,
                name: "b",
                sql: "CREATE TABLE b (id INTEGER PRIMARY KEY);",
            },
        ];
        let err = migrate_with(&mut db, &dup).unwrap_err();
        assert!(matches!(err, SqliteError::InvalidMigrationSet(_)));

        let non_pos = [Migration {
            version: 0,
            name: "zero",
            sql: "CREATE TABLE z (id INTEGER PRIMARY KEY);",
        }];
        let err = migrate_with(&mut db, &non_pos).unwrap_err();
        assert!(matches!(err, SqliteError::InvalidMigrationSet(_)));
    }

    #[test]
    fn concurrent_apply_rechecks_version_after_acquiring_write_lock() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("migration-race.sqlite3");
        let mut setup = Database::open(&path).unwrap();
        ensure_migrations_table(&mut setup).unwrap();
        drop(setup);

        let migration = Migration {
            version: 77,
            name: "concurrent_race",
            sql: "CREATE TABLE IF NOT EXISTS concurrent_race (id INTEGER PRIMARY KEY);",
        };
        let barrier = Arc::new(Barrier::new(2));
        let handles: Vec<_> = (0..2)
            .map(|_| {
                let mut db = Database::open(&path).unwrap();
                let migration = migration.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    assert!(load_applied(&db, migration.version).unwrap().is_none());
                    barrier.wait();
                    apply_one(&mut db, &migration)
                })
            })
            .collect();

        for result in handles.into_iter().map(|handle| handle.join().unwrap()) {
            result.expect("both migrators must converge after taking the write lock");
        }

        let db = Database::open(path).unwrap();
        let rows: i64 = db
            .connection()
            .query_row(
                "SELECT COUNT(*) FROM schema_migrations WHERE version = 77",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(rows, 1);
    }
}

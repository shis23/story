use std::sync::{Arc, Barrier};

use storyforge_infra_sqlite::Database;
use storyforge_infra_sqlite::migrations::{current_version, migrate};
use tempfile::TempDir;

#[test]
fn concurrent_first_start_migrations_are_idempotent() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("cold.sqlite3");
    let connections: Vec<_> = (0..8).map(|_| Database::open(&path).unwrap()).collect();
    let barrier = Arc::new(Barrier::new(connections.len()));
    let handles: Vec<_> = connections
        .into_iter()
        .map(|mut db| {
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                migrate(&mut db)
            })
        })
        .collect();

    for result in handles.into_iter().map(|handle| handle.join().unwrap()) {
        result.expect("every cold-start migrator must converge idempotently");
    }

    let db = Database::open(path).unwrap();
    assert_eq!(current_version(&db).unwrap(), 3);
    let rows: i64 = db
        .connection()
        .query_row("SELECT COUNT(*) FROM schema_migrations", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(rows, 3);
}

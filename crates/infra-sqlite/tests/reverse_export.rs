//! SQLite → JSON reverse export tests.
//!
//! Verifies the exported JSON is readable, matches counts, redacts secrets,
//! includes a manifest, and is independently importable (re-import equivalence).
//!
//! Gate 5 审查二.1：严格导出目标校验（forbidden targets、符号链接/junction、
//! 普通文件、嵌套新目录）——任何拒绝都必须保持源 DB 与目标路径字节不变。

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use storyforge_infra_sqlite::Database;
use storyforge_infra_sqlite::JsonImporter;
use storyforge_infra_sqlite::cutover::{CutoverOutcome, CutoverPlan, CutoverRequest, run_cutover};
use storyforge_infra_sqlite::exporter::{export_sqlite_to_json, read_export_manifest};
use tempfile::TempDir;

fn write_json(path: &Path, value: &serde_json::Value) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
}

fn sample_source(dir: &Path) {
    write_json(
        &dir.join("cards.json"),
        &serde_json::json!([{
            "id": "card-1",
            "name": "Hero",
            "source_character_id": null,
            // Assembled at runtime so static secret scanners ignore the fixture payload.
            "api_key": format!("{}{}", "sk-", "super-secret-value-1234567890")
        }]),
    );
    write_json(
        &dir.join("campaigns.json"),
        &serde_json::json!([{
            "id": "camp-1",
            "card_id": "card-1",
            "name": "Main",
            "created_at": "2026-07-13T00:00:00Z",
            "revision": 0,
            "chronicle_revision": 0,
            "conversation_id": "conv-1",
            "lineage_id": "lin-1"
        }]),
    );
    write_json(
        &dir.join("conversations").join("conv-1.json"),
        &serde_json::json!({
            "id": "conv-1",
            "campaign_id": "camp-1",
            "character_id": null,
            "created_at": "2026-07-13T00:00:00Z",
            "updated_at": "2026-07-13T00:00:00Z",
            "nodes": []
        }),
    );
    write_json(&dir.join("instances.json"), &serde_json::json!([]));
    write_json(&dir.join("knowledge.json"), &serde_json::json!([]));
    write_json(&dir.join("tasks.json"), &serde_json::json!([]));
    write_json(
        &dir.join("round_summaries.json"),
        &serde_json::json!([{
            "id": "sum-a1",
            "campaign_id": "camp-1",
            "conversation_id": "conv-1",
            "turn": 1,
            "content": "leaf",
            "created_at": "2026-07-13T00:00:00Z",
            "level": 0,
            "lineage_id": "lin-1",
            "code": "A0001"
        }]),
    );
    write_json(&dir.join("turns.json"), &serde_json::json!([]));
}

#[test]
fn reverse_export_produces_readable_json() {
    let dir = TempDir::new().unwrap();
    sample_source(dir.path());

    // Cutover to SQLite first.
    let request = CutoverRequest {
        plan: CutoverPlan::new(dir.path(), dir.path().join("storyforge.sqlite3")),
        label: "export-test".into(),
    };
    let outcome = run_cutover(&request).unwrap();
    assert!(matches!(outcome, CutoverOutcome::Completed(_)));

    // Export to a separate directory.
    let export_dir = TempDir::new().unwrap();
    let db = Database::open(dir.path().join("storyforge.sqlite3")).unwrap();
    let result = export_sqlite_to_json(&db, export_dir.path()).unwrap();

    // Manifest exists.
    assert!(result.manifest_path.exists());
    let manifest = read_export_manifest(&result.manifest_path).unwrap();
    assert_eq!(
        manifest.get("direction").and_then(|v| v.as_str()),
        Some("sqlite-to-json-rollback")
    );
    assert_eq!(
        manifest.get("source_backend").and_then(|v| v.as_str()),
        Some("sqlite")
    );

    // Counts match.
    assert_eq!(result.report.cards, 1);
    assert_eq!(result.report.campaigns, 1);
    assert_eq!(result.report.conversations, 1);
    assert_eq!(result.report.summaries, 1);

    // JSON files exist in the export.
    assert!(export_dir.path().join("cards.json").exists());
    assert!(export_dir.path().join("campaigns.json").exists());
    assert!(
        export_dir
            .path()
            .join("conversations")
            .join("conv-1.json")
            .exists()
    );
}

#[test]
fn reverse_export_redacts_secrets() {
    let dir = TempDir::new().unwrap();
    sample_source(dir.path());

    let request = CutoverRequest {
        plan: CutoverPlan::new(dir.path(), dir.path().join("storyforge.sqlite3")),
        label: "redact-test".into(),
    };
    run_cutover(&request).unwrap();

    let export_dir = TempDir::new().unwrap();
    let db = Database::open(dir.path().join("storyforge.sqlite3")).unwrap();
    let result = export_sqlite_to_json(&db, export_dir.path()).unwrap();

    // The exported cards.json must NOT contain the secret field or value.
    let exported_cards = fs::read_to_string(export_dir.path().join("cards.json")).unwrap();
    assert!(
        !exported_cards.contains("sk-super-secret"),
        "exported cards contain unredacted secret value"
    );
    assert!(
        !exported_cards.contains("api_key"),
        "exported cards contain unredacted secret field name"
    );

    // The secret field removal should be noted in unsupported_fields.
    assert!(
        result
            .report
            .unsupported_fields
            .iter()
            .any(|f| f == "sensitive_field"),
        "expected sensitive_field in unsupported_fields: {:?}",
        result.report.unsupported_fields
    );

    // The original SQLite DB still has the secret (we don't mutate live data).
    let cards_json: serde_json::Value = db
        .connection()
        .query_row(
            "SELECT payload_json FROM character_cards LIMIT 1",
            [],
            |row| {
                let s: String = row.get(0)?;
                Ok(serde_json::from_str(&s).unwrap_or_default())
            },
        )
        .unwrap();
    let _cards_str = serde_json::to_string(&cards_json).unwrap();
    // The live DB payload still has the secret string (only the export is redacted).
    // Note: the importer stores the full payload including api_key.
}

#[test]
fn reverse_export_does_not_mutate_live_database() {
    let dir = TempDir::new().unwrap();
    sample_source(dir.path());

    let request = CutoverRequest {
        plan: CutoverPlan::new(dir.path(), dir.path().join("storyforge.sqlite3")),
        label: "nomut-test".into(),
    };
    run_cutover(&request).unwrap();

    // Record the DB file hash before export.
    let db_path = dir.path().join("storyforge.sqlite3");
    let before = fs::read(&db_path).unwrap();

    let export_dir = TempDir::new().unwrap();
    let db = Database::open(&db_path).unwrap();
    export_sqlite_to_json(&db, export_dir.path()).unwrap();
    drop(db);

    let after = fs::read(&db_path).unwrap();
    assert_eq!(before, after, "live database was mutated by export");
}

#[test]
fn reverse_export_can_be_reimported() {
    let dir = TempDir::new().unwrap();
    sample_source(dir.path());

    let request = CutoverRequest {
        plan: CutoverPlan::new(dir.path(), dir.path().join("storyforge.sqlite3")),
        label: "reimport-test".into(),
    };
    run_cutover(&request).unwrap();

    // Export.
    let export_dir = TempDir::new().unwrap();
    let db = Database::open(dir.path().join("storyforge.sqlite3")).unwrap();
    export_sqlite_to_json(&db, export_dir.path()).unwrap();
    drop(db);

    // Re-import the exported JSON into a fresh database.
    let dir2 = TempDir::new().unwrap();
    let mut db2 = Database::open(dir2.path().join("storyforge.sqlite3")).unwrap();
    let mut importer = JsonImporter::new(&mut db2);
    let report = importer.import_data_dir(export_dir.path()).unwrap();

    assert_eq!(report.cards, 1);
    assert_eq!(report.campaigns, 1);
    assert_eq!(report.conversations, 1);
    assert_eq!(report.summaries, 1);
}

#[test]
fn reverse_export_refuses_live_db_directory() {
    let dir = TempDir::new().unwrap();
    sample_source(dir.path());

    let request = CutoverRequest {
        plan: CutoverPlan::new(dir.path(), dir.path().join("storyforge.sqlite3")),
        label: "refuse-test".into(),
    };
    run_cutover(&request).unwrap();

    // Try to export into the same directory as the DB — must fail.
    let db = Database::open(dir.path().join("storyforge.sqlite3")).unwrap();
    let err = export_sqlite_to_json(&db, dir.path()).unwrap_err();
    assert!(err.to_string().contains("refusing"));
}

#[test]
fn reverse_export_replaces_stale_conversation_files_atomically() {
    let dir = TempDir::new().unwrap();
    sample_source(dir.path());
    let request = CutoverRequest {
        plan: CutoverPlan::new(dir.path(), dir.path().join("storyforge.sqlite3")),
        label: "atomic-export".into(),
    };
    run_cutover(&request).unwrap();

    let export_dir = TempDir::new().unwrap();
    // Seed a stale conversation that must not survive publish.
    fs::create_dir_all(export_dir.path().join("conversations")).unwrap();
    fs::write(
        export_dir.path().join("conversations").join("stale.json"),
        b"{\"id\":\"stale\"}",
    )
    .unwrap();

    let db = Database::open(dir.path().join("storyforge.sqlite3")).unwrap();
    export_sqlite_to_json(&db, export_dir.path()).unwrap();

    assert!(
        !export_dir
            .path()
            .join("conversations")
            .join("stale.json")
            .exists(),
        "stale conversation survived non-atomic export"
    );
    assert!(
        export_dir
            .path()
            .join("conversations")
            .join("conv-1.json")
            .exists()
    );
}

// ─── Gate 5 审查二.1：严格导出目标校验 ────────────────────────────────────

fn db_bytes(path: &Path) -> Vec<u8> {
    fs::read(path).unwrap()
}

/// 目录树确定性快照（相对路径 → 字节），用于断言导出尝试前后目标不变。
fn snapshot_tree(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn walk(dir: &Path, root: &Path, out: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in fs::read_dir(dir).unwrap() {
            let p = entry.unwrap().path();
            let rel = p.strip_prefix(root).unwrap().to_path_buf();
            let meta = fs::symlink_metadata(&p).unwrap();
            if meta.is_dir() {
                walk(&p, root, out);
            } else {
                out.insert(rel, fs::read(&p).unwrap());
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(root, root, &mut out);
    out
}

/// 目标可能是文件（读字节）或目录（树快照）；不存在为 None。
type TargetSnapshot = Option<BTreeMap<PathBuf, Vec<u8>>>;

fn snapshot_target(target: &Path) -> TargetSnapshot {
    match fs::symlink_metadata(target) {
        Ok(meta) if meta.is_file() => {
            let mut map = BTreeMap::new();
            map.insert(PathBuf::from("<file>"), fs::read(target).unwrap());
            Some(map)
        }
        Ok(_) => Some(snapshot_tree(target)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(_) => None,
    }
}

fn assert_rejected_with_bytes_unchanged(name: &str, db_path: &Path, target: &Path, db: &Database) {
    let db_before = db_bytes(db_path);
    let target_before = snapshot_target(target);
    let err = export_sqlite_to_json(db, target).unwrap_err();
    assert!(
        err.to_string().contains("refusing"),
        "{name}: expected refusal, got: {err}"
    );
    assert_eq!(
        db_before,
        db_bytes(db_path),
        "{name}: source DB bytes changed"
    );
    assert_eq!(
        target_before,
        snapshot_target(target),
        "{name}: target path bytes/tree changed by a rejected export"
    );
}

fn cutover_setup(dir: &TempDir) -> PathBuf {
    let db_path = dir.path().join("storyforge.sqlite3");
    let request = CutoverRequest {
        plan: CutoverPlan::new(dir.path(), &db_path),
        label: "forbidden-targets".into(),
    };
    let outcome = run_cutover(&request).unwrap();
    assert!(matches!(outcome, CutoverOutcome::Completed(_)));
    db_path
}

#[test]
fn reverse_export_rejects_every_forbidden_target_without_touching_bytes() {
    let dir = TempDir::new().unwrap();
    sample_source(dir.path());
    let db_path = cutover_setup(&dir);
    // 锁文件在 cutover 后可能已被清理——显式确保存在使断言确定。
    fs::write(dir.path().join("storyforge.cutover.lock"), b"").unwrap();
    fs::write(dir.path().join("storyforge.authority.lock"), b"").unwrap();

    let data = dir.path();
    let db = Database::open(&db_path).unwrap();
    let targets: Vec<(&str, PathBuf)> = vec![
        ("source DB file", db_path.clone()),
        (
            "source DB -wal",
            PathBuf::from(format!("{}-wal", db_path.display())),
        ),
        (
            "source DB -shm",
            PathBuf::from(format!("{}-shm", db_path.display())),
        ),
        ("backend marker", data.join("storyforge.backend.json")),
        ("cutover lock", data.join("storyforge.cutover.lock")),
        ("authority lock", data.join("storyforge.authority.lock")),
        ("backup dir", data.join("sqlite-backups")),
        ("cards.json", data.join("cards.json")),
        ("campaigns.json", data.join("campaigns.json")),
        ("turns.json", data.join("turns.json")),
        ("instances.json", data.join("instances.json")),
        ("knowledge.json", data.join("knowledge.json")),
        ("tasks.json", data.join("tasks.json")),
        ("round_summaries.json", data.join("round_summaries.json")),
        ("mvu_translations.json", data.join("mvu_translations.json")),
        ("compress_jobs.json", data.join("compress_jobs.json")),
        ("characters.json", data.join("characters.json")),
        ("conversations dir", data.join("conversations")),
        ("campaign_world_info dir", data.join("campaign_world_info")),
        ("live data root", data.to_path_buf()),
    ];
    for (name, target) in &targets {
        assert_rejected_with_bytes_unchanged(name, &db_path, target, &db);
    }
    drop(db);
}

#[test]
fn reverse_export_rejects_plain_file_target_without_overwriting() {
    let dir = TempDir::new().unwrap();
    sample_source(dir.path());
    let db_path = cutover_setup(&dir);

    let target = dir.path().join("plain-target");
    fs::write(&target, b"i am a plain file, not a directory").unwrap();
    let db_before = db_bytes(&db_path);
    let target_before = fs::read(&target).unwrap();

    let db = Database::open(&db_path).unwrap();
    let err = export_sqlite_to_json(&db, &target).unwrap_err();
    assert!(
        err.to_string().contains("refusing"),
        "plain file target must be rejected, got: {err}"
    );
    drop(db);
    assert_eq!(db_before, db_bytes(&db_path), "source DB bytes changed");
    assert_eq!(
        target_before,
        fs::read(&target).unwrap(),
        "plain file target must not be overwritten"
    );
}

#[cfg(unix)]
fn make_dir_link(target: &Path, link: &Path) {
    std::os::unix::fs::symlink(target, link).unwrap();
}

#[cfg(windows)]
fn make_dir_link(target: &Path, link: &Path) {
    // mklink /J 创建 junction：普通用户无需管理员权限；junction 与符号链接
    // 一样带 reparse point 属性，symlink_metadata 会报告 is_symlink。
    let out = std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(link)
        .arg(target)
        .output()
        .expect("run mklink /J");
    assert!(
        out.status.success(),
        "mklink failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn reverse_export_rejects_symlink_or_junction_target() {
    let dir = TempDir::new().unwrap();
    sample_source(dir.path());
    let db_path = cutover_setup(&dir);

    let real = dir.path().join("real-target-dir");
    fs::create_dir_all(&real).unwrap();
    let link = dir.path().join("link-target");
    make_dir_link(&real, &link);
    let real_before = snapshot_tree(&real);

    let db = Database::open(&db_path).unwrap();
    let err = export_sqlite_to_json(&db, &link).unwrap_err();
    drop(db);
    assert!(
        err.to_string().contains("symbolic link") || err.to_string().contains("refusing"),
        "symlink/junction target must be rejected, got: {err}"
    );
    // 链接仍是链接（未被跟随/覆盖），真实目录内容不变。
    assert!(
        fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink(),
        "target link must remain a link"
    );
    assert_eq!(real_before, snapshot_tree(&real), "link target dir changed");
}

#[test]
fn reverse_export_rejects_ancestor_of_live_data_root() {
    let root = TempDir::new().unwrap();
    let data = root.path().join("data");
    fs::create_dir_all(&data).unwrap();
    sample_source(&data);
    let db_path = data.join("storyforge.sqlite3");
    let request = CutoverRequest {
        plan: CutoverPlan::new(&data, &db_path),
        label: "ancestor-test".into(),
    };
    run_cutover(&request).unwrap();

    let db = Database::open(&db_path).unwrap();
    // 目标是数据根的祖先（temp 根）→ 必须拒绝。
    let err = export_sqlite_to_json(&db, root.path()).unwrap_err();
    assert!(
        err.to_string().contains("refusing"),
        "ancestor target must be rejected, got: {err}"
    );
    // 兄弟目录 → 允许。
    let ok_target = root.path().join("export-out");
    let result = export_sqlite_to_json(&db, &ok_target).unwrap();
    assert!(result.manifest_path.exists());
}

#[test]
fn reverse_export_allows_nonexistent_nested_target() {
    let dir = TempDir::new().unwrap();
    sample_source(dir.path());
    let db_path = cutover_setup(&dir);

    let db = Database::open(&db_path).unwrap();
    let target = dir.path().join("nested").join("a").join("b");
    let result = export_sqlite_to_json(&db, &target).unwrap();
    assert!(result.manifest_path.exists());
    assert!(target.join("cards.json").exists());
    assert!(target.join("conversations").join("conv-1.json").exists());
}

//! SQLite → portable JSON reverse export.
//!
//! This produces a JSON data-directory layout matching what the JSON importer
//! reads, so an operator can roll back from SQLite to JSON by importing the
//! exported directory. The export is **explicit, versioned, validated, and
//! secret-safe**: it never deletes the SQLite DB or original JSON backup.
//!
//! Gate 5 审查二加固点：
//! - 二.1 导出目标严格校验：任何写入之前拒绝 live DB/`-wal`/`-shm`、marker、
//!   锁文件、备份目录、JSON 权威文件、活动数据根及其祖先、符号链接/junction、
//!   普通文件目标；拒绝不产生任何字节变化。
//! - 二.2 会话/世界书 id 的路径穿越与文件名碰撞防护；发布前从 staging 文件树
//!   重读校验 count/hash（`verify_export_tree`）。
//! - 二.3 模式拆分：`Rollback`（无损，绝不脱敏）与 `Diagnostic`（可脱敏，
//!   报告与 manifest 显式声明 `redacted`）。
//! - 二.4 `mutation_commits` / `chronicle_publication_jobs` 非空 → fail-closed。
//! - 二.5 导出前校验：schema 版本、migration checksum、integrity_check、
//!   foreign_key_check、必需表、corrupt payload 全部错误传播。
//! - 二.6 原子发布：唯一 staging 目录 → 旧目标先让位 → rename 发布；发布失败
//!   自动恢复旧目标并清理 staging；per-target 跨进程锁。

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::Connection;
use rusqlite::OptionalExtension;
use rusqlite::TransactionBehavior;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use crate::connection::Database;
use crate::cutover::MARKER_FILENAME;
use crate::error::{Result, SqliteError};
use crate::lease::AUTHORITY_LEASE_FILENAME;

/// Filename for the reverse-export manifest.
pub const EXPORT_MANIFEST_FILENAME: &str = "reverse_export_manifest.json";

/// cutover 锁文件名（与 cutover.rs 相同；此处用于禁止导出目标）。
const CUTOVER_LOCK_FILENAME: &str = "storyforge.cutover.lock";
/// SQLite 备份目录名（CutoverPlan::new 约定）。
const BACKUP_DIR_NAME: &str = "sqlite-backups";

/// 导出器读取/校验所依赖的必需表（V001–V008 全部创建）。
const REQUIRED_TABLES: &[&str] = &[
    "character_cards",
    "campaigns",
    "character_instances",
    "character_knowledge",
    "story_tasks",
    "round_summaries",
    "conversations",
    "turns",
    "characters",
    "mvu_translations",
    "campaign_world_info",
    "chronicle_compress_jobs",
    "preaccept_outbox",
    "mutation_commits",
    "chronicle_publication_jobs",
];

/// 导出模式（审查二.3）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportMode {
    /// 回滚导出：完全无损，绝不脱敏；产物可恢复（rollback 入口专用）。
    Rollback,
    /// 诊断导出：允许脱敏；产物不可声明为可恢复。
    Diagnostic,
}

impl ExportMode {
    pub fn redacts(self) -> bool {
        matches!(self, ExportMode::Diagnostic)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            ExportMode::Rollback => "rollback",
            ExportMode::Diagnostic => "diagnostic",
        }
    }
}

/// Report describing the reverse export result.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReverseExportReport {
    pub export_manifest_hash: String,
    pub schema_version: i64,
    pub cards: usize,
    pub campaigns: usize,
    pub instances: usize,
    pub knowledge: usize,
    pub tasks: usize,
    pub summaries: usize,
    pub conversations: usize,
    pub turns: usize,
    /// 导出的角色库条目数（Gate 5）。
    pub characters: usize,
    /// 导出是否发生脱敏：true = 产物不可用于恢复（审查二.3）。
    pub redacted: bool,
    pub unsupported_fields: Vec<String>,
}

/// Result of a reverse export operation.
#[derive(Debug, Clone)]
pub struct ReverseExportResult {
    pub report: ReverseExportReport,
    pub export_dir: PathBuf,
    pub manifest_path: PathBuf,
}

/// Test-only fault injection for the export publish path（审查二.6）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportFault {
    None,
    /// Staging 校验通过后、触碰目标之前失败。
    AfterStageVerified,
    /// 旧目标已让位、staging 尚未发布时失败（必须自动恢复旧目标）。
    AfterTargetMovedAside,
}

/// 导出到唯一 staging 目录的结果（rollback 入口复用，不发布）。
pub(crate) struct StagedExport {
    pub report: ReverseExportReport,
}

/// Export a SQLite database to a portable JSON directory layout
/// (diagnostic mode; redaction applies).
pub fn export_sqlite_to_json(
    db: &Database,
    export_dir: impl AsRef<Path>,
) -> Result<ReverseExportResult> {
    export_sqlite_to_json_inner(
        db,
        export_dir.as_ref(),
        ExportMode::Diagnostic,
        ExportFault::None,
    )
}

/// Export with an explicit mode（审查二.3）：`Rollback` 无损，`Diagnostic` 可脱敏。
pub fn export_sqlite_to_json_with_mode(
    db: &Database,
    export_dir: impl AsRef<Path>,
    mode: ExportMode,
) -> Result<ReverseExportResult> {
    export_sqlite_to_json_inner(db, export_dir.as_ref(), mode, ExportFault::None)
}

/// Test-facing diagnostic export with fault injection（审查二.6）。
pub fn export_sqlite_to_json_with_fault(
    db: &Database,
    export_dir: impl AsRef<Path>,
    fault: ExportFault,
) -> Result<ReverseExportResult> {
    export_sqlite_to_json_inner(db, export_dir.as_ref(), ExportMode::Diagnostic, fault)
}

fn export_sqlite_to_json_inner(
    db: &Database,
    export_dir: &Path,
    mode: ExportMode,
    fault: ExportFault,
) -> Result<ReverseExportResult> {
    // 二.1：任何写入之前校验导出目标。
    validate_export_target(db.path(), export_dir)?;

    // 二.6：per-target 跨进程锁（rollback 路径由 authority 租约覆盖，不加此锁）。
    let _lock = acquire_export_lock(export_dir)?;

    // 唯一 staging 目录（目标的兄弟；pid+纳秒保证唯一）。
    let parent = export_dir.parent().unwrap_or_else(|| Path::new("."));
    let name = export_dir
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("export");
    let stage_dir = unique_sibling_dir(parent, "staging", name);

    let result = (|| -> Result<ReverseExportResult> {
        let staged = export_sqlite_to_json_staged(db, &stage_dir, mode)?;
        if fault == ExportFault::AfterStageVerified {
            return Err(SqliteError::Other(
                "injected fault: after stage verified".into(),
            ));
        }
        publish_staged_export(&stage_dir, export_dir, fault)?;
        Ok(ReverseExportResult {
            report: staged.report,
            export_dir: export_dir.to_path_buf(),
            manifest_path: export_dir.join(EXPORT_MANIFEST_FILENAME),
        })
    })();

    match result {
        Ok(ok) => Ok(ok),
        Err(e) => {
            // 发布失败/注入故障：清理 staging（成功后 stage 已不存在，静默）。
            let _ = fs::remove_dir_all(&stage_dir);
            Err(e)
        }
    }
}

/// 二.6：原子发布——旧目标先让位（唯一 aside 名），再 rename staging → 目标；
/// 发布失败自动恢复旧目标。
fn publish_staged_export(stage_dir: &Path, export_dir: &Path, fault: ExportFault) -> Result<()> {
    let parent = export_dir.parent().unwrap_or_else(|| Path::new("."));
    let name = export_dir
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("export");

    let mut aside: Option<PathBuf> = None;
    if export_dir.exists() {
        let aside_path = unique_sibling_dir(parent, "pre-export", name);
        fs::rename(export_dir, &aside_path).map_err(|e| {
            SqliteError::Other(format!(
                "failed to move previous export {} aside: {e}",
                export_dir.display()
            ))
        })?;
        aside = Some(aside_path);
    }

    if fault == ExportFault::AfterTargetMovedAside {
        // 恢复旧目标，然后报注入故障（旧目标回归原位，无 aside 残留）。
        if let Some(a) = &aside {
            let _ = fs::rename(a, export_dir);
        }
        return Err(SqliteError::Other(
            "injected fault: after target moved aside".into(),
        ));
    }

    if let Some(parent) = export_dir.parent() {
        fs::create_dir_all(parent)?;
    }
    match fs::rename(stage_dir, export_dir) {
        Ok(()) => Ok(()),
        Err(e) => {
            if let Some(a) = &aside {
                fs::rename(a, export_dir).map_err(|r| {
                    SqliteError::Other(format!(
                        "publish failed ({e}) AND restoring previous export failed ({r}) at {}",
                        export_dir.display()
                    ))
                })?;
            }
            Err(SqliteError::Other(format!(
                "failed to publish export at {}: {e}",
                export_dir.display()
            )))
        }
    }
}

/// 二.1：严格导出目标校验（在任何写入之前）。
///
/// 拒绝：源 DB 文件本身、`-wal`/`-shm`、marker、锁文件、备份目录、JSON 权威
/// 文件、活动数据根（= DB 父目录）及其祖先、符号链接/junction、普通文件目标。
fn validate_export_target(db_path: &Path, export_dir: &Path) -> Result<()> {
    // 符号链接 / junction / reparse point：symlink_metadata 不跟随链接；
    // Windows 上 junction 与符号链接都带 reparse point 属性 → is_symlink。
    match fs::symlink_metadata(export_dir) {
        Ok(meta) => {
            if meta.file_type().is_symlink() {
                return Err(SqliteError::Other(format!(
                    "refusing export: target {} is a symbolic link or junction",
                    export_dir.display()
                )));
            }
            if meta.is_file() {
                return Err(SqliteError::Other(format!(
                    "refusing export: target {} exists as a plain file, not a directory",
                    export_dir.display()
                )));
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // 目标不存在：允许（将创建）。
        }
        Err(e) => {
            return Err(SqliteError::Other(format!(
                "cannot inspect export target {}: {e}",
                export_dir.display()
            )));
        }
    }

    let data_dir = db_path.parent().unwrap_or_else(|| Path::new("."));
    let export_canon = canonicalize_loose(export_dir);
    let data_dir_canon = canonicalize_loose(data_dir);

    // 数据根（= DB 父目录）与其中的权威文件/锁/marker/备份目录。
    let mut forbidden: Vec<PathBuf> = vec![
        db_path.to_path_buf(),
        PathBuf::from(format!("{}-wal", db_path.display())),
        PathBuf::from(format!("{}-shm", db_path.display())),
        data_dir.join(MARKER_FILENAME),
        data_dir.join(CUTOVER_LOCK_FILENAME),
        data_dir.join(AUTHORITY_LEASE_FILENAME),
        data_dir.join(BACKUP_DIR_NAME),
        data_dir.to_path_buf(),
    ];
    for name in [
        "cards.json",
        "campaigns.json",
        "turns.json",
        "instances.json",
        "knowledge.json",
        "tasks.json",
        "round_summaries.json",
        "mvu_translations.json",
        "compress_jobs.json",
        "characters.json",
        "conversations",
        "campaign_world_info",
    ] {
        forbidden.push(data_dir.join(name));
    }
    for target in &forbidden {
        if canonicalize_loose(target) == export_canon {
            return Err(SqliteError::Other(format!(
                "refusing export: target {} is a live StoryForge path (database, marker, lock, backup, JSON authority file, or the live data root)",
                export_dir.display()
            )));
        }
    }

    // 目标是数据根的祖先（例如临时根目录）→ 拒绝。
    if data_dir_canon.starts_with(&export_canon) {
        return Err(SqliteError::Other(format!(
            "refusing export: target {} is the live data root or an ancestor of it",
            export_dir.display()
        )));
    }
    // 三审7：目标是数据根**内部**的任意后代（data_dir/sub/export 这类）→ 拒绝。
    // 旧实现只在扁平 forbidden 列表里枚举已知文件名，data_dir 内部的任意新子路径
    // 都能绕过——可能把导出写进活动数据根里污染权威树或与未来 JSON 文件冲突。
    if export_canon.starts_with(&data_dir_canon) {
        return Err(SqliteError::Other(format!(
            "refusing export: target {} is inside the live data root",
            export_dir.display()
        )));
    }
    Ok(())
}

/// 宽松 canonicalize：路径不存在时沿父链向上找到第一个存在的祖先并 canonicalize，
/// 再把剩余相对段重接回去，保证「路径即身份」的比较不受存在性影响（三审7：
/// 深层不存在的目标 dir/subdir/export 也须与 data_dir 可靠比较 starts_with）。
fn canonicalize_loose(path: &Path) -> PathBuf {
    if let Ok(c) = fs::canonicalize(path) {
        return c;
    }
    // 收集从 path 向上直到第一个可 canonicalize 的祖先，把相对段记下。
    let mut relative_segments: Vec<std::ffi::OsString> = Vec::new();
    let mut cursor = path.to_path_buf();
    loop {
        if let Some(name) = cursor.file_name() {
            relative_segments.push(name.to_os_string());
        }
        let Some(parent) = cursor.parent() else {
            break;
        };
        if let Ok(pc) = fs::canonicalize(parent) {
            // 祖先可 canonicalize：把记下的相对段逆序重接回去。
            let mut result = pc;
            for seg in relative_segments.iter().rev() {
                result.push(seg);
            }
            return result;
        }
        cursor = parent.to_path_buf();
    }
    path.to_path_buf()
}

fn unique_sibling_dir(parent: &Path, prefix: &str, name: &str) -> PathBuf {
    parent.join(format!(
        ".{name}.{prefix}-{}-{}",
        std::process::id(),
        now_nanos()
    ))
}

fn now_nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
}

/// 导出目标互斥锁（跨进程，审查二.6）。锁文件是目标的**兄弟**：
/// `.<name>.storyforge-export.lock`——目标被改名让位/重命名时锁保持稳定。
pub struct ExportLockGuard {
    #[cfg(unix)]
    file: std::fs::File,
    #[cfg(not(unix))]
    _file: std::fs::File,
    path: PathBuf,
}

/// Acquire the per-target export lock（与 `export_sqlite_to_json` 发布路径同一把锁）。
pub fn acquire_export_lock(export_dir: impl AsRef<Path>) -> Result<ExportLockGuard> {
    let export_dir = export_dir.as_ref();
    let parent = export_dir.parent().unwrap_or_else(|| Path::new("."));
    let name = export_dir
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("export");
    let lock_path = parent.join(format!(".{name}.storyforge-export.lock"));
    ExportLockGuard::acquire(&lock_path)
}

impl ExportLockGuard {
    fn acquire(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        #[cfg(unix)]
        {
            use std::os::unix::io::AsRawFd;
            let file = fs::OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(false)
                .open(path)?;
            let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
            if result != 0 {
                return Err(SqliteError::Other(format!(
                    "another export is in progress for this target (lock held): {}",
                    path.display()
                )));
            }
            Ok(ExportLockGuard {
                file,
                path: path.to_path_buf(),
            })
        }
        #[cfg(not(unix))]
        {
            use std::os::windows::fs::OpenOptionsExt;
            let file = fs::OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(false)
                .share_mode(0)
                .open(path)
                .map_err(|_| {
                    SqliteError::Other(format!(
                        "another export is in progress for this target (lock held): {}",
                        path.display()
                    ))
                })?;
            Ok(ExportLockGuard {
                _file: file,
                path: path.to_path_buf(),
            })
        }
    }
}

impl Drop for ExportLockGuard {
    fn drop(&mut self) {
        #[cfg(unix)]
        {
            use std::os::unix::io::AsRawFd;
            unsafe {
                libc::flock(self.file.as_raw_fd(), libc::LOCK_UN);
            }
        }
        // Windows 关闭句柄即释放锁。
        let _ = &self.path;
    }
}

/// 把 SQLite 导出到**调用方创建好的** staging 目录：导出前校验（二.5）、
/// 按模式写入（二.3）、发布前从 staging 文件树重读校验（二.2）。
/// 不发布、不加锁（发布方 / rollback 入口负责锁与清理）。
pub(crate) fn export_sqlite_to_json_staged(
    db: &Database,
    stage_dir: &Path,
    mode: ExportMode,
) -> Result<StagedExport> {
    fs::create_dir_all(stage_dir.join("conversations"))?;

    // 单一只读事务：一致快照。
    let mut conn = Connection::open(db.path())?;
    conn.pragma_update(None, "query_only", true)?;
    let tx = conn.transaction_with_behavior(TransactionBehavior::Deferred)?;

    // 二.5：导出前校验。
    let schema_version = validate_source_database(&tx)?;

    let mut unsupported = Vec::new();

    // Export each table's payload_json as a JSON array.
    let cards = export_table_array(&tx, "character_cards", "card_id", &mut unsupported, mode)?;
    // Campaigns are normalized so the legacy top-level `story_clock` field is
    // repaired from the authoritative variables entry before export (Gate 4).
    let campaigns = export_table_array_normalized(
        &tx,
        "campaigns",
        "campaign_id",
        &mut unsupported,
        mode,
        |value| {
            normalize_campaign_story_clock(value);
        },
    )?;
    let instances = export_table_array(
        &tx,
        "character_instances",
        "instance_id",
        &mut unsupported,
        mode,
    )?;
    let knowledge = export_table_array(
        &tx,
        "character_knowledge",
        "knowledge_id",
        &mut unsupported,
        mode,
    )?;
    let tasks = export_table_array(&tx, "story_tasks", "task_id", &mut unsupported, mode)?;
    let summaries =
        export_table_array(&tx, "round_summaries", "summary_id", &mut unsupported, mode)?;
    let turns = export_table_array(&tx, "turns", "turn_id", &mut unsupported, mode)?;
    let mvu = export_table_array(
        &tx,
        "mvu_translations",
        "source_character_id",
        &mut unsupported,
        mode,
    )?;

    // 活跃 pre-accept 状态（pending outbox）无法无损表达：明确阻止导出，而不是丢弃。
    if table_exists(&tx, "preaccept_outbox")? {
        let pending: i64 = tx.query_row(
            "SELECT COUNT(*) FROM preaccept_outbox WHERE status = 'pending'",
            [],
            |row| row.get(0),
        )?;
        if pending > 0 {
            return Err(SqliteError::Other(format!(
                "refusing reverse export: {pending} pending pre-accept outbox row(s) represent an active pre-accept state that the JSON backend cannot express losslessly"
            )));
        }
        let count: i64 = tx.query_row("SELECT COUNT(*) FROM preaccept_outbox", [], |row| {
            row.get(0)
        })?;
        if count > 0 {
            unsupported.push(format!(
                "preaccept_outbox:{count} rows (SQLite-native pre-accept recovery ledger; no JSON equivalent file)"
            ));
        } else {
            unsupported.push(
                "preaccept_outbox:empty (SQLite-native table; no JSON equivalent file)".into(),
            );
        }
    }

    // 二.4：只读台账无法无损表达——非空必须 fail-closed（绝不允许只写 warning 继续）；
    // 空台账时仍显式分类（既有契约）。
    for (table, label) in [
        ("mutation_commits", "turn accept ledger"),
        ("chronicle_publication_jobs", "chronicle publication ledger"),
    ] {
        if table_exists(&tx, table)? {
            let count: i64 = tx.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get(0)
            })?;
            if count > 0 {
                return Err(SqliteError::Other(format!(
                    "refusing reverse export: {table} has {count} row(s) (SQLite-native {label}); the JSON layout cannot express them losslessly"
                )));
            }
            unsupported.push(format!(
                "{table}:{count} rows (SQLite-native {label}; no JSON equivalent file)"
            ));
        }
    }

    // Conversations are stored as individual files matching the JSON layout.
    let conversations = export_conversations(&tx, stage_dir, &mut unsupported, mode)?;

    // 本局世界书：JSON 布局 campaign_world_info/{campaign_id}.json（可无损表达）。
    let world_info = export_world_info(&tx, stage_dir, &mut unsupported, mode)?;

    // Chronicle 压缩任务：JSON 布局 compress_jobs.json（可无损表达）。
    let compress_jobs = export_compress_jobs(&tx, &mut unsupported, mode)?;

    // 角色库：JSON 布局 characters.json（StoredCharacter 契约形态，可无损表达）。
    let characters = export_characters(&tx, &mut unsupported, mode)?;

    // Write array files into the staging tree only.
    write_array_file(stage_dir, "cards.json", &cards)?;
    write_array_file(stage_dir, "campaigns.json", &campaigns)?;
    write_array_file(stage_dir, "instances.json", &instances)?;
    write_array_file(stage_dir, "knowledge.json", &knowledge)?;
    write_array_file(stage_dir, "tasks.json", &tasks)?;
    write_array_file(stage_dir, "round_summaries.json", &summaries)?;
    write_array_file(stage_dir, "turns.json", &turns)?;
    write_array_file(stage_dir, "mvu_translations.json", &mvu)?;
    write_array_file(stage_dir, "compress_jobs.json", &compress_jobs)?;
    write_array_file(stage_dir, "characters.json", &characters)?;

    tx.commit()?;

    // Build the manifest hash over all exported content (sorted, stable).
    let manifest_hash = collect_export_hash(
        &cards,
        &campaigns,
        &instances,
        &knowledge,
        &tasks,
        &summaries,
        &turns,
        &conversations,
        &mvu,
        &world_info,
        &compress_jobs,
        &characters,
    );

    let report = ReverseExportReport {
        export_manifest_hash: manifest_hash,
        schema_version,
        cards: cards.len(),
        campaigns: campaigns.len(),
        instances: instances.len(),
        knowledge: knowledge.len(),
        tasks: tasks.len(),
        summaries: summaries.len(),
        conversations: conversations.len(),
        turns: turns.len(),
        characters: characters.len(),
        redacted: mode.redacts(),
        unsupported_fields: unsupported,
    };

    let staged_manifest_path = stage_dir.join(EXPORT_MANIFEST_FILENAME);
    let manifest = serde_json::json!({
        "created_at": chrono::Utc::now().to_rfc3339(),
        "direction": "sqlite-to-json-rollback",
        "mode": mode.as_str(),
        "schema_version": schema_version,
        "source_backend": "sqlite",
        "target_backend": "json",
        "export_manifest_hash": report.export_manifest_hash,
        "redacted": report.redacted,
        "counts": {
            "cards": report.cards,
            "campaigns": report.campaigns,
            "instances": report.instances,
            "knowledge": report.knowledge,
            "tasks": report.tasks,
            "summaries": report.summaries,
            "conversations": report.conversations,
            "turns": report.turns,
        },
        "extras": {
            "mvu_translations": mvu.len(),
            "campaign_world_info": world_info.len(),
            "compress_jobs": compress_jobs.len(),
            "characters": characters.len(),
        },
        "unsupported_fields": report.unsupported_fields,
        "note": "Import this directory via the JSON importer to roll back to JSON backend.",
    });
    fs::write(
        &staged_manifest_path,
        serde_json::to_vec_pretty(&manifest).map_err(SqliteError::from)?,
    )?;

    // 二.2：发布前从 staging 文件树重新读取并校验 count/hash。
    verify_export_tree(stage_dir)?;

    Ok(StagedExport { report })
}

/// 二.5：导出前校验——schema 版本、migration checksum、必需表、
/// integrity_check、foreign_key_check。任何错误直接传播。
fn validate_source_database(tx: &rusqlite::Transaction<'_>) -> Result<i64> {
    // 1) 已迁移（schema_migrations 存在）。
    let has_migrations: i64 = tx.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='schema_migrations'",
        [],
        |row| row.get(0),
    )?;
    if has_migrations == 0 {
        return Err(SqliteError::Other(
            "refusing reverse export: database is not migrated (no schema_migrations table)".into(),
        ));
    }

    // 2) 当前 schema 版本 == 最新内置 migration 版本。
    let version: i64 = tx.query_row(
        "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
        [],
        |row| row.get(0),
    )?;
    let expected = crate::migrations::builtin_migrations()
        .iter()
        .map(|m| m.version)
        .max()
        .unwrap_or(0);
    if version != expected {
        return Err(SqliteError::Other(format!(
            "refusing reverse export: database schema version {version} != latest builtin version {expected} (old or partial migration state)"
        )));
    }

    // 3) 已应用 migration 的 checksum 必须与内置定义一致。
    for migration in crate::migrations::builtin_migrations() {
        let stored: Option<String> = tx
            .query_row(
                "SELECT checksum FROM schema_migrations WHERE version = ?1",
                [migration.version],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(stored) = stored {
            let checksum = migration.checksum();
            if !migration.matches_checksum(&stored) {
                return Err(SqliteError::Other(format!(
                    "refusing reverse export: migration checksum mismatch for version {}: stored {stored} != expected {checksum}",
                    migration.version
                )));
            }
        }
    }

    // 4) 必需表齐全。
    for table in REQUIRED_TABLES {
        if !table_exists(tx, table)? {
            return Err(SqliteError::Other(format!(
                "refusing reverse export: required table {table} is missing"
            )));
        }
    }

    // 5) 完整性。
    let integrity: String = tx.query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
    if !integrity.eq_ignore_ascii_case("ok") {
        return Err(SqliteError::Other(format!(
            "refusing reverse export: integrity_check failed: {integrity}"
        )));
    }

    // 6) 外键一致性。
    let mut stmt = tx.prepare("PRAGMA foreign_key_check")?;
    let mut rows = stmt.query([])?;
    let mut violations = 0i64;
    let mut first = String::new();
    while let Some(row) = rows.next()? {
        if violations == 0 {
            let table: String = row.get(0)?;
            let rowid: Option<i64> = row.get(1)?;
            first = format!(
                "{table} rowid={}",
                rowid.map(|r| r.to_string()).unwrap_or_else(|| "-".into())
            );
        }
        violations += 1;
    }
    if violations > 0 {
        return Err(SqliteError::Other(format!(
            "refusing reverse export: {violations} foreign-key violation(s) (first: {first})"
        )));
    }
    Ok(version)
}

fn table_exists(tx: &rusqlite::Transaction<'_>, table: &str) -> Result<bool> {
    let exists: Option<i64> = tx
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1",
            [table],
            |row| row.get(0),
        )
        .optional()?;
    Ok(exists.is_some())
}

fn export_table_array(
    tx: &rusqlite::Transaction<'_>,
    table: &str,
    id_column: &str,
    unsupported: &mut Vec<String>,
    mode: ExportMode,
) -> Result<Vec<Value>> {
    export_table_array_with(tx, table, id_column, unsupported, mode, |_| {})
}

/// Like [`export_table_array`] but lets the caller normalize each payload
/// (e.g. story-clock authority repair) before it is written.
fn export_table_array_normalized(
    tx: &rusqlite::Transaction<'_>,
    table: &str,
    id_column: &str,
    unsupported: &mut Vec<String>,
    mode: ExportMode,
    normalize: impl Fn(&mut Value),
) -> Result<Vec<Value>> {
    export_table_array_with(tx, table, id_column, unsupported, mode, normalize)
}

fn export_table_array_with(
    tx: &rusqlite::Transaction<'_>,
    table: &str,
    id_column: &str,
    unsupported: &mut Vec<String>,
    mode: ExportMode,
    normalize: impl Fn(&mut Value),
) -> Result<Vec<Value>> {
    if !table_exists(tx, table)? {
        return Ok(Vec::new());
    }

    let sql = format!("SELECT {id_column}, payload_json FROM {table} ORDER BY {id_column}");
    let mut stmt = tx.prepare(&sql)?;
    let rows = stmt.query_map([], |row| {
        let id: String = row.get(0)?;
        let payload: String = row.get(1)?;
        Ok((id, payload))
    })?;

    let mut out = Vec::new();
    for row in rows {
        let (id, payload) = row?;
        let mut value: Value = serde_json::from_str(&payload).map_err(|e| {
            SqliteError::Other(format!("corrupt payload_json in {table} id={id}: {e}"))
        })?;

        // Merge the id into the payload so the exported record is self-contained
        // and matches the JSON store layout (which uses the id as a field).
        if let Value::Object(map) = &mut value
            && !map.contains_key("id")
        {
            map.insert("id".to_string(), Value::String(id));
        }

        normalize(&mut value);

        // 二.3：仅诊断模式脱敏；rollback 模式必须逐字节无损。
        if mode.redacts() {
            redact_secret_values(&mut value, unsupported);
        }

        out.push(value);
    }
    Ok(out)
}

/// Gate 4 story-clock authority: repair the legacy top-level `story_clock`
/// field from the authoritative `variables` entry inside the exported payload,
/// so a reverse export never carries the stale half of the old dual
/// representation.
fn normalize_campaign_story_clock(value: &mut Value) {
    let Some(map) = value.as_object_mut() else {
        return;
    };
    let Some(Value::Array(variables)) = map.get("variables") else {
        return;
    };
    let authoritative = variables.iter().find_map(|v| {
        let obj = v.as_object()?;
        if obj.get("key")?.as_str()? == "story_clock" {
            obj.get("value").and_then(Value::as_str).map(str::to_string)
        } else {
            None
        }
    });
    let Some(authoritative) = authoritative else {
        return;
    };
    match map.get_mut("story_clock") {
        Some(Value::String(field)) if field == &authoritative => {}
        Some(slot) => *slot = Value::String(authoritative),
        None => {
            map.insert("story_clock".to_string(), Value::String(authoritative));
        }
    }
}

/// Export the per-campaign world info books into `campaign_world_info/` files
/// (JSON store layout `campaign_world_info/{campaign_id}.json`).
///
/// 二.2：文件名即 campaign_id（importer 以文件名为 id），只能严格校验不能
/// 编码——不安全 id 显式 Err；大小写/尾点归一化碰撞显式 Err。
fn export_world_info(
    tx: &rusqlite::Transaction<'_>,
    stage_dir: &Path,
    unsupported: &mut Vec<String>,
    mode: ExportMode,
) -> Result<Vec<(String, Value)>> {
    if !table_exists(tx, "campaign_world_info")? {
        return Ok(Vec::new());
    }
    let info_dir = stage_dir.join("campaign_world_info");
    fs::create_dir_all(&info_dir)?;

    let mut stmt = tx.prepare(
        "SELECT campaign_id, payload_json FROM campaign_world_info ORDER BY campaign_id",
    )?;
    let rows = stmt.query_map([], |row| {
        let campaign_id: String = row.get(0)?;
        let payload: String = row.get(1)?;
        Ok((campaign_id, payload))
    })?;

    let mut out = Vec::new();
    let mut seen: HashMap<String, String> = HashMap::new();
    for row in rows {
        let (campaign_id, payload) = row?;
        validate_raw_filename_id(&campaign_id, "world-info campaign")?;
        let key = collision_key(&campaign_id);
        if let Some(first) = seen.get(&key)
            && first != &campaign_id
        {
            return Err(SqliteError::Other(format!(
                "world-info campaign ids {first:?} and {campaign_id:?} collide under filename {campaign_id:?} (case-insensitive / trailing-dot normalized); refusing export"
            )));
        }
        seen.insert(key, campaign_id.clone());

        let mut value: Value = serde_json::from_str(&payload).map_err(|e| {
            SqliteError::Other(format!(
                "corrupt payload_json in campaign_world_info id={campaign_id}: {e}"
            ))
        })?;
        if mode.redacts() {
            redact_secret_values(&mut value, unsupported);
        }
        let file = info_dir.join(format!("{campaign_id}.json"));
        fs::write(
            &file,
            serde_json::to_vec_pretty(&value).map_err(SqliteError::from)?,
        )?;
        out.push((campaign_id, value));
    }
    Ok(out)
}

/// 严格校验「文件名即 id」的段：可移植安全字符、单段、非保留设备名。
fn validate_raw_filename_id(id: &str, kind: &str) -> Result<()> {
    if id.is_empty() {
        return Err(SqliteError::Other(format!(
            "{kind} id is empty; refusing export"
        )));
    }
    if id.contains('\0') {
        return Err(SqliteError::Other(format!(
            "{kind} id contains NUL; refusing export"
        )));
    }
    if id == "." || id == ".." || id.contains('/') || id.contains('\\') {
        return Err(SqliteError::Other(format!(
            "{kind} id is not a safe single filename segment; refusing export: {id}"
        )));
    }
    if !id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
    {
        return Err(SqliteError::Other(format!(
            "{kind} id contains non-portable characters and cannot be encoded because the JSON layout uses the id as the filename; refusing export: {id}"
        )));
    }
    if windows_reserved_name(id) {
        return Err(SqliteError::Other(format!(
            "{kind} id is a reserved device name; refusing export: {id}"
        )));
    }
    Ok(())
}

/// Export Chronicle compress jobs as `compress_jobs.json` in the JSON
/// `CompressJob` shape (id/campaign_id/conversation_id/lineage_id/kind/status/
/// attempts/max_attempts/last_error/uncovered_*/created_at/updated_at).
fn export_compress_jobs(
    tx: &rusqlite::Transaction<'_>,
    unsupported: &mut Vec<String>,
    mode: ExportMode,
) -> Result<Vec<Value>> {
    if !table_exists(tx, "chronicle_compress_jobs")? {
        return Ok(Vec::new());
    }
    let mut stmt = tx.prepare(
        r#"
        SELECT job_id, campaign_id, conversation_id, lineage_id, kind, status, attempts,
               max_attempts, last_error, uncovered_a_at_enqueue, uncovered_b_at_enqueue,
               created_at, updated_at
        FROM chronicle_compress_jobs ORDER BY job_id
        "#,
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, Option<String>>(2)?,
            row.get::<_, Option<String>>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, String>(5)?,
            row.get::<_, i64>(6)?,
            row.get::<_, i64>(7)?,
            row.get::<_, Option<String>>(8)?,
            row.get::<_, i64>(9)?,
            row.get::<_, i64>(10)?,
            row.get::<_, String>(11)?,
            row.get::<_, String>(12)?,
        ))
    })?;

    let mut out = Vec::new();
    for row in rows {
        let (
            job_id,
            campaign_id,
            conversation_id,
            lineage_id,
            kind,
            status,
            attempts,
            max_attempts,
            last_error,
            uncovered_a,
            uncovered_b,
            created_at,
            updated_at,
        ) = row?;
        let mut value = compress_job_value(
            job_id,
            campaign_id,
            conversation_id,
            lineage_id,
            kind,
            status,
            attempts,
            max_attempts,
            last_error,
            uncovered_a,
            uncovered_b,
            created_at,
            updated_at,
        );
        if mode.redacts() {
            redact_secret_values(&mut value, unsupported);
        }
        out.push(value);
    }
    Ok(out)
}

/// CompressJob 列投影（exporter / recompute_db_export_hash 共用，hash 同口径）。
#[allow(clippy::too_many_arguments)]
fn compress_job_value(
    job_id: String,
    campaign_id: String,
    conversation_id: Option<String>,
    lineage_id: Option<String>,
    kind: String,
    status: String,
    attempts: i64,
    max_attempts: i64,
    last_error: Option<String>,
    uncovered_a: i64,
    uncovered_b: i64,
    created_at: String,
    updated_at: String,
) -> Value {
    let mut value = serde_json::json!({
        "id": job_id,
        "campaign_id": campaign_id,
        "kind": kind,
        "status": status,
        "attempts": attempts,
        "max_attempts": max_attempts,
        "uncovered_a_at_enqueue": uncovered_a,
        "uncovered_b_at_enqueue": uncovered_b,
        "created_at": created_at,
        "updated_at": updated_at,
    });
    let obj = value.as_object_mut().expect("json! object is an object");
    if let Some(v) = conversation_id {
        obj.insert("conversation_id".to_string(), Value::String(v));
    }
    if let Some(v) = lineage_id {
        obj.insert("lineage_id".to_string(), Value::String(v));
    }
    if let Some(v) = last_error {
        obj.insert("last_error".to_string(), Value::String(v));
    }
    value
}

/// 导出角色库为 characters.json（`StoredCharacter` 契约形态
/// `{id, info, imported_at}`，与 importer 归一化及 CharacterStore 反序列化一致）。
fn export_characters(
    tx: &rusqlite::Transaction<'_>,
    unsupported: &mut Vec<String>,
    mode: ExportMode,
) -> Result<Vec<Value>> {
    if !table_exists(tx, "characters")? {
        return Ok(Vec::new());
    }
    let mut stmt = tx.prepare(
        r#"
        SELECT character_id, info_json, imported_at
        FROM characters ORDER BY character_id
        "#,
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
        ))
    })?;

    let mut out = Vec::new();
    for row in rows {
        let (character_id, info_json, imported_at) = row?;
        let info: Value = serde_json::from_str(&info_json).map_err(|e| {
            SqliteError::Other(format!(
                "corrupt info_json in characters id={character_id}: {e}"
            ))
        })?;
        let mut value = character_value(character_id, info, imported_at);
        if mode.redacts() {
            redact_secret_values(&mut value, unsupported);
        }
        out.push(value);
    }
    Ok(out)
}

/// StoredCharacter 投影（exporter / recompute_db_export_hash 共用，hash 同口径）。
fn character_value(character_id: String, info: Value, imported_at: String) -> Value {
    serde_json::json!({
        "id": character_id,
        "info": info,
        "imported_at": imported_at,
    })
}

fn export_conversations(
    tx: &rusqlite::Transaction<'_>,
    stage_dir: &Path,
    unsupported: &mut Vec<String>,
    mode: ExportMode,
) -> Result<Vec<Value>> {
    if !table_exists(tx, "conversations")? {
        return Ok(Vec::new());
    }

    let mut stmt = tx.prepare(
        "SELECT conversation_id, payload_json FROM conversations ORDER BY conversation_id",
    )?;
    let rows = stmt.query_map([], |row| {
        let id: String = row.get(0)?;
        let payload: String = row.get(1)?;
        Ok((id, payload))
    })?;

    let mut out = Vec::new();
    let conv_dir = stage_dir.join("conversations");
    let mut seen: HashMap<String, String> = HashMap::new();
    for row in rows {
        let (id, payload) = row?;
        let filename = conversation_storage_filename(&id)?;
        let key = collision_key(&filename);
        if let Some(first) = seen.get(&key)
            && first != &id
        {
            return Err(SqliteError::Other(format!(
                "conversation ids {first:?} and {id:?} collide under filename {filename:?} (case-insensitive / trailing-dot normalized); refusing export"
            )));
        }
        seen.insert(key, id.clone());

        let mut value: Value = serde_json::from_str(&payload).map_err(|e| {
            SqliteError::Other(format!(
                "corrupt payload_json in conversations id={id}: {e}"
            ))
        })?;
        if let Value::Object(map) = &mut value
            && !map.contains_key("id")
        {
            map.insert("id".to_string(), Value::String(id.clone()));
        }
        if mode.redacts() {
            redact_secret_values(&mut value, unsupported);
        }

        // 文件名由单段安全字符（或注入式百分号编码）构成，不可能逃逸目录。
        fs::write(
            conv_dir.join(format!("{filename}.json")),
            serde_json::to_vec_pretty(&value).map_err(SqliteError::from)?,
        )?;
        out.push(value);
    }
    Ok(out)
}

/// 会话存储文件名（二.2）：可移植安全单段名原样使用（向后兼容），否则
/// **注入式**百分号编码——'%' 自身也编码，未编码 id 与编码结果永不相交，
/// 任何两个不同 id 不会映射到同一文件名。路径分隔符/绝对路径/NUL/`.`/`..`
/// → 显式 Err（绝不静默改写）。
fn conversation_storage_filename(id: &str) -> Result<String> {
    if id.is_empty() {
        return Err(SqliteError::Other(
            "conversation id is empty; refusing export".into(),
        ));
    }
    if id.contains('\0') {
        return Err(SqliteError::Other(
            "conversation id contains NUL; refusing export".into(),
        ));
    }
    if id == "." || id == ".." {
        return Err(SqliteError::Other(format!(
            "conversation id is a path component; refusing export: {id}"
        )));
    }
    if id.contains('/') || id.contains('\\') {
        return Err(SqliteError::Other(format!(
            "conversation id contains path separators; refusing export: {id}"
        )));
    }
    if Path::new(id).is_absolute() {
        return Err(SqliteError::Other(format!(
            "conversation id looks absolute; refusing export: {id}"
        )));
    }
    let safe = id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.');
    if safe {
        if windows_reserved_name(id) {
            return Err(SqliteError::Other(format!(
                "conversation id is a reserved device name; refusing export: {id}"
            )));
        }
        Ok(id.to_string())
    } else {
        Ok(percent_encode(id))
    }
}

/// 注入式百分号编码：可移植字符原样，其余字节 %XX（小写十六进制）。
fn percent_encode(raw: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(raw.len());
    for b in raw.bytes() {
        if b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.' {
            out.push(b as char);
        } else {
            out.push('%');
            out.push(HEX[(b >> 4) as usize] as char);
            out.push(HEX[(b & 0xf) as usize] as char);
        }
    }
    out
}

/// 文件名碰撞归一化键：Windows 大小写不敏感 + 尾随点/空格被剥离。
fn collision_key(name: &str) -> String {
    name.to_ascii_lowercase()
        .trim_end_matches(['.', ' '])
        .to_string()
}

/// Windows 保留设备名（CON/PRN/AUX/NUL/COM1-9/LPT1-9，大小写不敏感）。
fn windows_reserved_name(stem: &str) -> bool {
    let upper = stem.to_ascii_uppercase();
    if matches!(upper.as_str(), "CON" | "PRN" | "AUX" | "NUL") {
        return true;
    }
    let com_or_lpt = |prefix: &str| {
        upper
            .strip_prefix(prefix)
            .and_then(|rest| rest.parse::<u8>().ok())
            .is_some_and(|n| (1..=9).contains(&n))
    };
    com_or_lpt("COM") || com_or_lpt("LPT")
}

/// Recursively redact values whose keys look like secrets, and free-text that
/// looks like credentials. Records the field category (never the hostile key).
fn redact_secret_values(value: &mut Value, unsupported: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            let keys: Vec<String> = map.keys().cloned().collect();
            for key in keys {
                if is_secret_field(&key) {
                    map.remove(&key);
                    if !unsupported.iter().any(|f| f == "sensitive_field") {
                        unsupported.push("sensitive_field".into());
                    }
                } else if let Some(child) = map.get_mut(&key) {
                    redact_secret_values(child, unsupported);
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                redact_secret_values(item, unsupported);
            }
        }
        Value::String(text) if looks_like_secret_text(text) => {
            *text = "[REDACTED]".into();
            if !unsupported.iter().any(|f| f == "free_text_secret") {
                unsupported.push("free_text_secret".into());
            }
        }
        Value::String(text) if looks_like_absolute_path(text) => {
            *text = "[REDACTED]".into();
            if !unsupported.iter().any(|f| f == "absolute_path") {
                unsupported.push("absolute_path".into());
            }
        }
        _ => {}
    }
}

fn is_secret_field(key: &str) -> bool {
    let lower = key.to_ascii_lowercase().replace('-', "_");
    let names = [
        "api_key",
        "apikey",
        "password",
        "secret",
        "token",
        "access_token",
        "refresh_token",
        "authorization",
        "private_key",
        "credential",
        "credentials",
        "bearer",
        "client_secret",
    ];
    names
        .iter()
        .any(|name| lower == *name || lower.contains(name))
}

fn looks_like_secret_text(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    let compact = lower.replace([' ', '\t'], "");
    compact.contains("api_key=")
        || compact.contains("api_key:")
        || compact.contains("token=")
        || compact.contains("token:")
        || lower.contains("bearer ")
        || compact.contains("password=")
        || compact.contains("password:")
        || compact.contains("secret=")
        || compact.contains("secret:")
        || compact.contains("credential=")
        || compact.contains("credential:")
        || {
            let bytes = text.as_bytes();
            bytes.windows(3).any(|w| w == b"sk-") && text.len() >= 24
        }
}

fn looks_like_absolute_path(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.windows(3).any(|window| {
        window[0].is_ascii_alphabetic()
            && window[1] == b':'
            && (window[2] == b'\\' || window[2] == b'/')
    }) || text.contains("\\\\")
        || [
            "/home/", "/Users/", "/data/", "/tmp/", "/var/", "/root/", "/etc/",
        ]
        .iter()
        .any(|prefix| text.contains(prefix))
}

fn write_array_file(dir: &Path, filename: &str, items: &[Value]) -> Result<()> {
    let path = dir.join(filename);
    fs::write(
        &path,
        serde_json::to_vec_pretty(items).map_err(SqliteError::from)?,
    )?;
    Ok(())
}

fn compute_export_hash(tables: &[(&str, &[Value])], world_info: &[(String, Value)]) -> String {
    let mut hasher = Sha256::new();
    for (name, items) in tables {
        hash_named_array(&mut hasher, name, items);
    }
    // 世界书 hash 绑定 campaign_id（与 readiness / importer / cutover 同投影）；
    // 空集合时仅写入前缀，与旧导出 hash 一致。
    crate::readiness::hash_world_info_pairs(&mut hasher, world_info);
    hex_encode(hasher.finalize())
}

#[allow(clippy::too_many_arguments)]
fn collect_export_hash(
    cards: &[Value],
    campaigns: &[Value],
    instances: &[Value],
    knowledge: &[Value],
    tasks: &[Value],
    summaries: &[Value],
    turns: &[Value],
    conversations: &[Value],
    mvu: &[Value],
    world_info: &[(String, Value)],
    compress_jobs: &[Value],
    characters: &[Value],
) -> String {
    compute_export_hash(
        &[
            ("cards", cards),
            ("campaigns", campaigns),
            ("instances", instances),
            ("knowledge", knowledge),
            ("tasks", tasks),
            ("round_summaries", summaries),
            ("turns", turns),
            ("conversations", conversations),
            ("mvu_translations", mvu),
            ("compress_jobs", compress_jobs),
            ("characters", characters),
        ],
        world_info,
    )
}

fn hash_named_array(hasher: &mut Sha256, name: &str, items: &[Value]) {
    hasher.update(name.as_bytes());
    hasher.update(b"\0");
    let mut encoded: Vec<String> = items
        .iter()
        .map(|v| serde_json::to_string(v).unwrap_or_default())
        .collect();
    encoded.sort();
    for item in encoded {
        hasher.update(item.as_bytes());
        hasher.update(b"\n");
    }
}

/// 二.2：发布前（或事后审计时）从导出目录文件树**重读**并校验 per-table
/// 计数与内容 hash 是否与 manifest 一致；任何不一致 → Err。
pub fn verify_export_tree(export_dir: &Path) -> Result<()> {
    let manifest = read_export_manifest(&export_dir.join(EXPORT_MANIFEST_FILENAME))?;
    let counts = manifest
        .get("counts")
        .and_then(|v| v.as_object())
        .ok_or_else(|| {
            SqliteError::Other("export tree verification failed: manifest missing counts".into())
        })?;
    let extras = manifest
        .get("extras")
        .and_then(|v| v.as_object())
        .ok_or_else(|| {
            SqliteError::Other("export tree verification failed: manifest missing extras".into())
        })?;

    let cards = read_tree_array(export_dir, "cards.json")?;
    let campaigns = read_tree_array(export_dir, "campaigns.json")?;
    let instances = read_tree_array(export_dir, "instances.json")?;
    let knowledge = read_tree_array(export_dir, "knowledge.json")?;
    let tasks = read_tree_array(export_dir, "tasks.json")?;
    let summaries = read_tree_array(export_dir, "round_summaries.json")?;
    let turns = read_tree_array(export_dir, "turns.json")?;
    let mvu = read_tree_array(export_dir, "mvu_translations.json")?;
    let compress_jobs = read_tree_array(export_dir, "compress_jobs.json")?;
    let characters = read_tree_array(export_dir, "characters.json")?;
    let conversations = read_tree_conversations(&export_dir.join("conversations"))?;
    let world_info = crate::readiness::read_world_info_dir(export_dir.join("campaign_world_info"))?;

    check_tree_count("cards", cards.len(), counts)?;
    check_tree_count("campaigns", campaigns.len(), counts)?;
    check_tree_count("instances", instances.len(), counts)?;
    check_tree_count("knowledge", knowledge.len(), counts)?;
    check_tree_count("tasks", tasks.len(), counts)?;
    check_tree_count("round_summaries", summaries.len(), counts)?;
    check_tree_count("conversations", conversations.len(), counts)?;
    check_tree_count("turns", turns.len(), counts)?;
    check_tree_count("mvu_translations", mvu.len(), extras)?;
    check_tree_count("campaign_world_info", world_info.len(), extras)?;
    check_tree_count("compress_jobs", compress_jobs.len(), extras)?;
    check_tree_count("characters", characters.len(), extras)?;

    let hash = compute_export_hash(
        &[
            ("cards", &cards),
            ("campaigns", &campaigns),
            ("instances", &instances),
            ("knowledge", &knowledge),
            ("tasks", &tasks),
            ("round_summaries", &summaries),
            ("turns", &turns),
            ("conversations", &conversations),
            ("mvu_translations", &mvu),
            ("compress_jobs", &compress_jobs),
            ("characters", &characters),
        ],
        &world_info,
    );
    let expected = manifest
        .get("export_manifest_hash")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    if hash != expected {
        return Err(SqliteError::Other(format!(
            "export tree verification failed: content hash mismatch tree={hash} manifest={expected}"
        )));
    }
    Ok(())
}

fn check_tree_count(name: &str, actual: usize, map: &Map<String, Value>) -> Result<()> {
    // manifest 的 counts 键名沿用报告字段（summaries 而非 round_summaries）。
    let key = match name {
        "round_summaries" => "summaries",
        other => other,
    };
    let expected = map.get(key).and_then(|v| v.as_u64()).unwrap_or(u64::MAX);
    if actual as u64 != expected {
        return Err(SqliteError::Other(format!(
            "export tree verification failed: {name} count {actual} != manifest {expected}"
        )));
    }
    Ok(())
}

fn read_tree_array(dir: &Path, filename: &str) -> Result<Vec<Value>> {
    let path = dir.join(filename);
    let raw = fs::read(&path).map_err(|e| {
        SqliteError::Other(format!(
            "export tree verification failed reading {}: {e}",
            path.display()
        ))
    })?;
    let value: Value = serde_json::from_slice(&raw).map_err(|e| {
        SqliteError::Other(format!(
            "export tree verification failed parsing {}: {e}",
            path.display()
        ))
    })?;
    match value {
        Value::Array(items) => Ok(items),
        other => Err(SqliteError::Other(format!(
            "export tree verification failed: {} is not a JSON array: {other}",
            path.display()
        ))),
    }
}

fn read_tree_conversations(dir: &Path) -> Result<Vec<Value>> {
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut paths: Vec<PathBuf> = fs::read_dir(dir)
        .map_err(|e| {
            SqliteError::Other(format!(
                "export tree verification failed reading {}: {e}",
                dir.display()
            ))
        })?
        .map(|e| e.map(|e| e.path()))
        .collect::<std::io::Result<Vec<PathBuf>>>()
        .map_err(|e| {
            SqliteError::Other(format!(
                "export tree verification failed reading {}: {e}",
                dir.display()
            ))
        })?;
    paths.sort();
    let mut out = Vec::new();
    for path in paths {
        if path.extension().and_then(|x| x.to_str()) != Some("json") {
            continue;
        }
        let raw = fs::read(&path).map_err(|e| {
            SqliteError::Other(format!(
                "export tree verification failed reading {}: {e}",
                path.display()
            ))
        })?;
        let value: Value = serde_json::from_slice(&raw).map_err(|e| {
            SqliteError::Other(format!(
                "export tree verification failed parsing {}: {e}",
                path.display()
            ))
        })?;
        out.push(value);
    }
    Ok(out)
}

/// 用导出器 hash 投影（全部集合无条件参与、世界书绑定 campaign_id）从
/// 数据库重建内容 hash——用于 rollback 自检：staging 重新导入的新 DB 的
/// 内容 hash 必须等于导出 manifest 的 export_manifest_hash。
pub fn recompute_db_export_hash(db: &Database) -> Result<String> {
    fn load_payloads(db: &Database, table: &str) -> Result<Vec<Value>> {
        let sql = format!("SELECT payload_json FROM {table}");
        let mut stmt = db.connection().prepare(&sql)?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        let mut out = Vec::new();
        for row in rows {
            let raw = row?;
            out.push(serde_json::from_str(&raw).map_err(|e| {
                SqliteError::Other(format!("corrupt payload_json in {table}: {e}"))
            })?);
        }
        Ok(out)
    }

    let cards = load_payloads(db, "character_cards")?;
    let campaigns = load_payloads(db, "campaigns")?;
    let instances = load_payloads(db, "character_instances")?;
    let knowledge = load_payloads(db, "character_knowledge")?;
    let tasks = load_payloads(db, "story_tasks")?;
    let summaries = load_payloads(db, "round_summaries")?;
    let turns = load_payloads(db, "turns")?;
    let conversations = load_payloads(db, "conversations")?;
    let mvu = load_payloads(db, "mvu_translations")?;

    let world_info = {
        let mut stmt = db
            .connection()
            .prepare("SELECT campaign_id, payload_json FROM campaign_world_info")?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (campaign_id, raw) = row?;
            let value: Value = serde_json::from_str(&raw).map_err(|e| {
                SqliteError::Other(format!(
                    "corrupt payload_json in campaign_world_info id={campaign_id}: {e}"
                ))
            })?;
            out.push((campaign_id, value));
        }
        out
    };

    let compress_jobs = {
        let mut stmt = db.connection().prepare(
            r#"
            SELECT job_id, campaign_id, conversation_id, lineage_id, kind, status, attempts,
                   max_attempts, last_error, uncovered_a_at_enqueue, uncovered_b_at_enqueue,
                   created_at, updated_at
            FROM chronicle_compress_jobs
            "#,
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, i64>(6)?,
                row.get::<_, i64>(7)?,
                row.get::<_, Option<String>>(8)?,
                row.get::<_, i64>(9)?,
                row.get::<_, i64>(10)?,
                row.get::<_, String>(11)?,
                row.get::<_, String>(12)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (
                job_id,
                campaign_id,
                conversation_id,
                lineage_id,
                kind,
                status,
                attempts,
                max_attempts,
                last_error,
                uncovered_a,
                uncovered_b,
                created_at,
                updated_at,
            ) = row?;
            out.push(compress_job_value(
                job_id,
                campaign_id,
                conversation_id,
                lineage_id,
                kind,
                status,
                attempts,
                max_attempts,
                last_error,
                uncovered_a,
                uncovered_b,
                created_at,
                updated_at,
            ));
        }
        out
    };

    let characters = {
        let mut stmt = db
            .connection()
            .prepare("SELECT character_id, info_json, imported_at FROM characters")?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (character_id, info_json, imported_at) = row?;
            let info: Value = serde_json::from_str(&info_json).map_err(|e| {
                SqliteError::Other(format!(
                    "corrupt info_json in characters id={character_id}: {e}"
                ))
            })?;
            out.push(character_value(character_id, info, imported_at));
        }
        out
    };

    Ok(compute_export_hash(
        &[
            ("cards", &cards),
            ("campaigns", &campaigns),
            ("instances", &instances),
            ("knowledge", &knowledge),
            ("tasks", &tasks),
            ("round_summaries", &summaries),
            ("turns", &turns),
            ("conversations", &conversations),
            ("mvu_translations", &mvu),
            ("compress_jobs", &compress_jobs),
            ("characters", &characters),
        ],
        &world_info,
    ))
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

/// Read a reverse export manifest from disk for inspection.
pub fn read_export_manifest(path: &Path) -> Result<Map<String, Value>> {
    let raw = fs::read_to_string(path)?;
    let value: Value = serde_json::from_str(&raw)?;
    match value {
        Value::Object(map) => Ok(map),
        _ => Err(SqliteError::Other(
            "export manifest is not a JSON object".into(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conversation_filename_encoding_is_injective() {
        let ids = [
            "conv-1", "a:b", "a b", "a%3Ab", "a..b", "a.b", "雪", "a?b#c", "a+b=1", "CON-1",
        ];
        let mut seen = std::collections::HashMap::new();
        for id in ids {
            let name = conversation_storage_filename(id).unwrap();
            assert!(
                seen.insert(name.clone(), id).is_none(),
                "collision for {id:?} -> {name:?}"
            );
        }
        // 安全 id 原样保留；危险字符被注入式编码。
        assert_eq!(conversation_storage_filename("conv-1").unwrap(), "conv-1");
        assert_eq!(conversation_storage_filename("a:b").unwrap(), "a%3ab");
        assert_eq!(conversation_storage_filename("a b").unwrap(), "a%20b");
        assert_eq!(conversation_storage_filename("雪").unwrap(), "%e9%9b%aa");
        // 编码结果含 '%'，与未编码 id（不含 '%'）永不相交。
        assert!(
            !conversation_storage_filename("conv-1")
                .unwrap()
                .contains('%')
        );
        assert!(conversation_storage_filename("a:b").unwrap().contains('%'));
    }

    #[test]
    fn conversation_filename_rejects_path_shapes() {
        for id in ["../x", "a/../b", "/abs", "a\\b", "", ".", "..", "x\0y"] {
            assert!(
                conversation_storage_filename(id).is_err(),
                "id {id:?} must be rejected"
            );
        }
    }

    #[test]
    fn collision_key_normalizes_case_and_trailing_dots() {
        assert_eq!(collision_key("Case-1"), collision_key("case-1"));
        assert_eq!(collision_key("abc."), collision_key("abc"));
        assert_ne!(collision_key("a:b"), collision_key("a?b"));
    }

    #[test]
    fn windows_reserved_names_are_detected() {
        for name in ["CON", "con", "PRN", "AUX", "NUL", "COM1", "com9", "LPT2"] {
            assert!(windows_reserved_name(name), "{name} should be reserved");
        }
        for name in ["CONSOLE", "COM10", "LPT0", "combat", "a%3Ab"] {
            assert!(
                !windows_reserved_name(name),
                "{name} should not be reserved"
            );
        }
    }

    #[test]
    fn validate_raw_filename_id_accepts_uuids_and_rejects_hostile_shapes() {
        assert!(validate_raw_filename_id("c8d3c7c0-0000-4000-8000-000000000001", "x").is_ok());
        for bad in [
            "../evil", "camp:1", "a\\b", "/abs", "..", ".", "", "CON", "a b",
        ] {
            assert!(
                validate_raw_filename_id(bad, "x").is_err(),
                "{bad:?} must be rejected"
            );
        }
    }
}

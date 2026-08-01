//! 生产 rollback 入口：SQLite 权威 → JSON 权威（审查二.7）。
//!
//! 精确序列：
//! 1. 获取 `data_dir` 的 EXCLUSIVE authority 租约——覆盖整个 rollback，
//!    阻塞任何并发 JSON/SQLite 写者进程（Wave-1 lease）。
//! 2. 复查 marker：必须 `SqliteAuthoritative`；Absent / Stale / JsonAuthoritative → Err。
//! 3. 导出前校验（二.5）+ **无损** rollback 模式导出到唯一 staging 目录
//!    （数据根的兄弟，**不是**活动数据根；导出器内部还做 staging 树重读校验）。
//! 4. staging 自检：把 staging 重新导入全新内存 DB，比较计数与内容 hash
//!    （导出器投影）并断言 manifest 声明零脱敏——证明产物可恢复。
//! 5. 显式确认：`confirm` 必须为 true；否则 Err（无任何状态变更）。
//! 6. 最后原子写 `JsonAuthoritative` marker（tmp + fsync + rename + fsync；
//!    unix 上额外 fsync 父目录）。SQLite DB 文件**不删除**——只有 marker 切换。
//!
//! 任何步骤失败：marker 不动（SQLite 仍权威）、staging 清理、DB 文件字节不变。
//! 成功后的 staging 目录即 rollback 导出产物（`RollbackReport::export_dir`），
//! 操作者可将其作为新的 JSON 数据目录采纳（布局与 JSON importer 完全一致）。

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Map, Value};

use crate::connection::Database;
use crate::cutover::{BackendMarker, CutoverPlan, MarkerStatus, inspect_marker};
use crate::error::{Result, SqliteError};
use crate::exporter::{self, EXPORT_MANIFEST_FILENAME, ExportMode};
use crate::importer::JsonImporter;
use crate::lease::AuthorityLeaseGuard;

/// Rollback 请求。
#[derive(Debug, Clone)]
pub struct RollbackRequest {
    pub plan: CutoverPlan,
    /// 显式确认：false 时任何步骤都不会改变 marker（无状态变更）。
    pub confirm: bool,
}

/// Rollback 结果（无路径之外的信息；产物目录供操作者采纳）。
#[derive(Debug, Clone)]
pub struct RollbackReport {
    /// 已验证、可恢复的 rollback 导出目录（= staging 产物，rollback 后保留）。
    pub export_dir: PathBuf,
    pub manifest_path: PathBuf,
    pub export_manifest_hash: String,
    pub schema_version: i64,
    pub redacted: bool,
    pub cards: usize,
    pub campaigns: usize,
    pub instances: usize,
    pub knowledge: usize,
    pub tasks: usize,
    pub summaries: usize,
    pub conversations: usize,
    pub turns: usize,
    pub characters: usize,
}

/// Test-only fault injection for the rollback（审查二.7）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RollbackFault {
    None,
    /// 无损导出完成后立即失败（自检之前）。
    AfterExport,
    /// 自检通过后、confirm / marker 之前失败。
    AfterSelfCheck,
}

/// Run the production rollback（SQLite 权威 → JSON 权威）。
pub fn run_rollback(request: &RollbackRequest) -> Result<RollbackReport> {
    run_rollback_with_fault(request, RollbackFault::None)
}

/// Run the rollback with a test-only fault injection point.
pub fn run_rollback_with_fault(
    request: &RollbackRequest,
    fault: RollbackFault,
) -> Result<RollbackReport> {
    let plan = &request.plan;

    // ── Step 1: EXCLUSIVE authority 租约覆盖整个 rollback ─────────────
    let _lease = AuthorityLeaseGuard::acquire_exclusive_in(&plan.data_dir)?;

    // ── Step 2: 复查 marker ───────────────────────────────────────────
    match inspect_marker(plan) {
        MarkerStatus::SqliteAuthoritative { .. } => {}
        MarkerStatus::JsonAuthoritative => {
            return Err(SqliteError::Other(
                "rollback requires SqliteAuthoritative marker; found JsonAuthoritative (rollback already completed)".into(),
            ));
        }
        MarkerStatus::Absent => {
            return Err(SqliteError::Other(
                "rollback requires SqliteAuthoritative marker; found Absent".into(),
            ));
        }
        MarkerStatus::Stale { reason } => {
            return Err(SqliteError::Other(format!(
                "rollback requires SqliteAuthoritative marker; found Stale: {reason}"
            )));
        }
    }

    // ── Step 3: 无损导出到唯一 staging 目录（数据根兄弟，非活动数据根）──
    let stage_dir = unique_rollback_stage_dir(plan);
    fs::create_dir_all(&stage_dir)?;

    let outcome = (|| -> Result<exporter::StagedExport> {
        let db = Database::open(&plan.db_path)?;
        let staged = exporter::export_sqlite_to_json_staged(&db, &stage_dir, ExportMode::Rollback)?;
        drop(db);

        if fault == RollbackFault::AfterExport {
            return Err(SqliteError::Other(
                "injected fault: after lossless export".into(),
            ));
        }

        // ── Step 4: staging 自检（可恢复性证明）──────────────────────
        verify_rollback_export(&stage_dir)?;

        if fault == RollbackFault::AfterSelfCheck {
            return Err(SqliteError::Other(
                "injected fault: after self-check".into(),
            ));
        }

        // ── Step 5: 显式确认 ─────────────────────────────────────────
        if !request.confirm {
            return Err(SqliteError::Other(
                "rollback requires explicit confirm=true; no state was changed".into(),
            ));
        }

        // ── Step 6: 最后写 JsonAuthoritative marker（提交点）──────────
        write_json_authoritative_marker(plan)?;

        Ok(staged)
    })();

    match outcome {
        Ok(staged) => {
            let manifest_path = stage_dir.join(EXPORT_MANIFEST_FILENAME);
            Ok(RollbackReport {
                export_dir: stage_dir,
                manifest_path,
                export_manifest_hash: staged.report.export_manifest_hash.clone(),
                schema_version: staged.report.schema_version,
                redacted: staged.report.redacted,
                cards: staged.report.cards,
                campaigns: staged.report.campaigns,
                instances: staged.report.instances,
                knowledge: staged.report.knowledge,
                tasks: staged.report.tasks,
                summaries: staged.report.summaries,
                conversations: staged.report.conversations,
                turns: staged.report.turns,
                characters: staged.report.characters,
            })
        }
        Err(e) => {
            // 任何失败：清理 staging（SQLite 仍权威，marker 未动）。
            let _ = fs::remove_dir_all(&stage_dir);
            Err(e)
        }
    }
}

/// 自检：把 rollback staging 重新导入全新内存 DB，比较计数与内容 hash
/// （导出器投影），并断言 manifest 声明 mode=rollback 且零脱敏。
pub fn verify_rollback_export(stage_dir: impl AsRef<Path>) -> Result<()> {
    let stage_dir = stage_dir.as_ref();
    let manifest = exporter::read_export_manifest(&stage_dir.join(EXPORT_MANIFEST_FILENAME))?;

    let mode = manifest
        .get("mode")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    if mode != "rollback" {
        return Err(SqliteError::Other(format!(
            "rollback self-check failed: manifest mode={mode:?} is not rollback"
        )));
    }
    let redacted = manifest
        .get("redacted")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    if redacted {
        return Err(SqliteError::Other(
            "rollback self-check failed: manifest declares redacted=true; export is not restorable"
                .into(),
        ));
    }

    let counts = manifest
        .get("counts")
        .and_then(|v| v.as_object())
        .cloned()
        .unwrap_or_default();
    let extras = manifest
        .get("extras")
        .and_then(|v| v.as_object())
        .cloned()
        .unwrap_or_default();

    let mut db = Database::open_in_memory()?;
    let report = JsonImporter::new(&mut db).import_data_dir(stage_dir)?;

    check_self_count("cards", report.cards, &counts)?;
    check_self_count("campaigns", report.campaigns, &counts)?;
    check_self_count("instances", report.instances, &counts)?;
    check_self_count("knowledge", report.knowledge, &counts)?;
    check_self_count("tasks", report.tasks, &counts)?;
    check_self_count("round_summaries", report.summaries, &counts)?;
    check_self_count("conversations", report.conversations, &counts)?;
    check_self_count("turns", report.turns, &counts)?;
    check_self_count("mvu_translations", report.mvu_translations, &extras)?;
    check_self_count("campaign_world_info", report.world_info, &extras)?;
    check_self_count("compress_jobs", report.compress_jobs, &extras)?;
    check_self_count("characters", report.characters, &extras)?;

    let hash = exporter::recompute_db_export_hash(&db)?;
    let expected = manifest
        .get("export_manifest_hash")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    if hash != expected {
        return Err(SqliteError::Other(format!(
            "rollback self-check content hash mismatch: reimported={hash}, manifest={expected}"
        )));
    }
    Ok(())
}

fn check_self_count(name: &str, actual: usize, map: &Map<String, Value>) -> Result<()> {
    // manifest 的 counts 键名沿用报告字段（summaries 而非 round_summaries）。
    let key = match name {
        "round_summaries" => "summaries",
        other => other,
    };
    let expected = map.get(key).and_then(|v| v.as_u64()).unwrap_or(u64::MAX);
    if actual as u64 != expected {
        return Err(SqliteError::Other(format!(
            "rollback self-check {name} count mismatch: manifest={expected}, reimported={actual}"
        )));
    }
    Ok(())
}

/// 最后写 JsonAuthoritative marker：tmp + fsync + rename + fsync；
/// unix 上额外 fsync 父目录（与 cutover 的 marker 写入同一持久化模式）。
fn write_json_authoritative_marker(plan: &CutoverPlan) -> Result<()> {
    let marker = BackendMarker::json_authoritative();
    let content = serde_json::to_vec_pretty(&marker)?;
    let marker_path = plan.marker_path();
    let tmp_path = marker_path.with_extension("json.tmp");
    fs::write(&tmp_path, &content).map_err(|e| {
        SqliteError::Other(format!("failed to write json-authoritative marker: {e}"))
    })?;
    fsync_file(&tmp_path).map_err(|e| {
        SqliteError::Other(format!("failed to fsync json-authoritative marker: {e}"))
    })?;
    fs::rename(&tmp_path, &marker_path).map_err(|e| {
        SqliteError::Other(format!("failed to publish json-authoritative marker: {e}"))
    })?;
    fsync_file(&marker_path).map_err(|e| {
        SqliteError::Other(format!("failed to fsync json-authoritative marker: {e}"))
    })?;
    fsync_parent_dir(&marker_path);
    Ok(())
}

/// fsync 文件（Windows 上 FlushFileBuffers 需要写访问，因此用写方式打开）。
fn fsync_file(path: &Path) -> Result<()> {
    let file = fs::OpenOptions::new().write(true).open(path)?;
    file.sync_all()?;
    Ok(())
}

/// fsync 父目录使 rename 本身持久化；Windows 不能对目录 fsync → unix only。
#[cfg(unix)]
fn fsync_parent_dir(path: &Path) {
    if let Some(parent) = path.parent() {
        if let Ok(dir) = fs::File::open(parent) {
            let _ = dir.sync_all();
        }
    }
}

#[cfg(not(unix))]
fn fsync_parent_dir(_path: &Path) {}

/// 唯一 rollback staging 目录（数据根兄弟：`.<name>.rollback-<pid>-<nanos>`）。
fn unique_rollback_stage_dir(plan: &CutoverPlan) -> PathBuf {
    let parent = plan.data_dir.parent().unwrap_or_else(|| Path::new("."));
    let name = plan
        .data_dir
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("data");
    parent.join(format!(
        ".rollback-{name}-{}-{}",
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

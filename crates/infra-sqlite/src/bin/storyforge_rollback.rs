//! Operator rollback CLI: SQLite authority → JSON authority (三审8).
//!
//! 受保护的生产入口，把 SQLite 权威回滚为 JSON 权威（导出 → 自检 → 原子安装
//! JSON 进 data_dir → 写 JsonAuthoritative marker）。整个流程在 EXCLUSIVE
//! authority 租约内，阻塞任何并发写者进程。
//!
//! Usage:
//!   storyforge_rollback <data_dir> [--confirm]
//!
//! - 无 `--confirm`：dry-run 校验（marker 必须 SqliteAuthoritative + 可导出 +
//!   自检通过），打印将要做的事，**不写 marker / 不装 JSON**。
//! - `--confirm`：执行真实 rollback。
//!
//! 退出码：0 成功；非 0 失败（marker/DB 未变，DB 字节不变）。
//!
//! DB 文件路径沿用 app 约定 `<data_dir>/storyforge.sqlite3`（SQLITE_DB_FILENAME）。

use std::env;
use std::path::PathBuf;
use std::process::ExitCode;

use storyforge_infra_sqlite::Database;
use storyforge_infra_sqlite::cutover::{CutoverPlan, MarkerStatus, inspect_marker};
use storyforge_infra_sqlite::exporter::{ExportMode, export_sqlite_to_json_with_mode};
use storyforge_infra_sqlite::rollback::{
    RollbackFault, RollbackRequest, run_rollback_with_fault, verify_rollback_export,
};

const DB_FILENAME: &str = "storyforge.sqlite3";

fn main() -> ExitCode {
    let mut args = env::args().skip(1);
    let Some(data_dir_raw) = args.next() else {
        eprintln!("usage: storyforge_rollback <data_dir> [--confirm]");
        return ExitCode::from(2);
    };
    let data_dir = PathBuf::from(&data_dir_raw);
    let confirm = args.any(|a| a == "--confirm");

    if !data_dir.is_dir() {
        eprintln!(
            "data_dir does not exist or is not a directory: {}",
            data_dir.display()
        );
        return ExitCode::from(2);
    }
    let db_path = data_dir.join(DB_FILENAME);
    let plan = CutoverPlan::new(&data_dir, &db_path);

    eprintln!("[rollback] data_dir = {}", data_dir.display());
    eprintln!("[rollback] db_path  = {}", db_path.display());
    eprintln!("[rollback] confirm  = {confirm}");

    // Dry-run 前置校验：marker 必须 SqliteAuthoritative，DB 可打开、可无损导出 +
    // 自检通过。dry-run 不写 marker、不装 JSON（确认产物可恢复才提示操作者加 --confirm）。
    let stage_dir = data_dir
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .join(format!(
            ".rollback-dryrun-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
    if let Err(e) = std::fs::create_dir_all(&stage_dir) {
        eprintln!("[rollback] dry-run staging dir create failed: {e}");
        return ExitCode::from(1);
    }
    let dry_run_ok = (|| -> Result<(), String> {
        match inspect_marker(&plan) {
            MarkerStatus::SqliteAuthoritative { .. } => {}
            other => {
                return Err(format!(
                    "marker is not SqliteAuthoritative (got {other:?}); rollback aborted"
                ));
            }
        }
        let db = Database::open(&db_path).map_err(|e| format!("open db: {e}"))?;
        let exported = export_sqlite_to_json_with_mode(&db, &stage_dir, ExportMode::Rollback)
            .map_err(|e| format!("lossless export: {e}"))?;
        drop(db);
        verify_rollback_export(&stage_dir).map_err(|e| format!("self-check: {e}"))?;
        eprintln!(
            "[rollback] dry-run OK: cards={}, campaigns={}, conversations={}, turns={}, hash={}",
            exported.report.cards,
            exported.report.campaigns,
            exported.report.conversations,
            exported.report.turns,
            exported.report.export_manifest_hash
        );
        Ok(())
    })();
    let _ = std::fs::remove_dir_all(&stage_dir);
    if let Err(e) = dry_run_ok {
        eprintln!("[rollback] dry-run FAILED: {e}");
        return ExitCode::from(1);
    }

    if !confirm {
        eprintln!("[rollback] dry-run passed; re-run with --confirm to perform the rollback");
        return ExitCode::SUCCESS;
    }

    eprintln!("[rollback] performing rollback (SQLite → JSON authority)...");
    // run_rollback 内部获取 EXCLUSIVE 租约、复查 marker、无损导出、自检、原子安装
    // JSON、最后写 marker。这里用 run_rollback_with_fault(None) 复用同一入口。
    match run_rollback_with_fault(
        &RollbackRequest {
            plan: plan.clone(),
            confirm: true,
        },
        RollbackFault::None,
    ) {
        Ok(report) => {
            eprintln!(
                "[rollback] SUCCESS: marker=JsonAuthoritative, export_dir={}, \
                 cards={}, campaigns={}, conversations={}, turns={}",
                report.export_dir.display(),
                report.cards,
                report.campaigns,
                report.conversations,
                report.turns
            );
            // 证明：回滚后 marker 确实翻成 JSON 权威。
            match inspect_marker(&plan) {
                MarkerStatus::JsonAuthoritative => {
                    eprintln!("[rollback] verified: marker is JsonAuthoritative");
                    ExitCode::SUCCESS
                }
                other => {
                    eprintln!(
                        "[rollback] INTERNAL ERROR: marker not JsonAuthoritative after success: {other:?}"
                    );
                    ExitCode::from(1)
                }
            }
        }
        Err(e) => {
            eprintln!("[rollback] FAILED: {e}");
            eprintln!("[rollback] marker/DB unchanged; resolve and retry");
            ExitCode::from(1)
        }
    }
}

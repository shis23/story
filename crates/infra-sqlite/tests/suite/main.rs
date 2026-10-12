//! infra-sqlite 的集成测试合成一个测试程序（每个文件一个模块），整个 crate 只链接一次。
//!
//! 需要独占进程的留在 `tests/` 顶层、各自一个程序：
//! - `backend_selection.rs` 会改 `STORYFORGE_STORAGE_BACKEND` 环境变量；
//! - `platform_locking.rs` 用 `current_exe() --exact <测试名>` 把自己当子进程再起一次。
//!
//! 只跑其中一个文件：`cargo test -p storyforge-infra-sqlite --test suite <文件名>::`。

mod audit_snapshot;
mod chronicle_publication;
mod cutover;
mod gate5_bigdata_perf;
mod gate5_export_safety;
mod gate5_fault_matrix;
mod gate5_migration_matrix;
mod gate5_reverse_export_roundtrip;
mod importer_diagnostics;
mod marker_reconcile;
mod migration_concurrency;
mod migration_readiness;
mod preaccept_lifecycle;
mod production_uow;
mod reverse_export;
mod reverse_export_gate4;
mod rollback;

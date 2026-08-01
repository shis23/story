//! StoryForge SQLite 迁移基础。
//!
//! 本 crate **默认不启用**为生产后端。它提供：
//! - 连接与 PRAGMA 管理
//! - schema_migrations + v1 schema
//! - UnitOfWork 事务封装
//! - 幂等 JSON importer（只读源 JSON）
//! - Store contract 测试基线
//!
//! 设计见 `docs/adr/0001-sqlite-migration-foundation.md`。

pub mod audit;
pub mod backend;
pub mod connection;
pub mod contract;
pub mod cutover;
pub mod error;
pub mod exporter;
pub mod importer;
pub mod lease;
pub mod migrations;
pub mod preaccept;
pub mod production;
pub mod publication;
pub mod readiness;
pub mod rollback;
pub mod unit_of_work;

pub use audit::{SqliteAuditSnapshot, capture_audit_snapshot};
pub use backend::{
    BackendDiagnostics, BackendSelection, BackendSelectionError, BackendSource,
    DEFAULT_BACKEND_ENV_VAR, PinnedBackend, StorageBackend,
};
pub use connection::{Database, STORYFORGE_APPLICATION_ID};
pub use cutover::{
    BackendMarker, CutoverDiagnostics, CutoverFault, CutoverOutcome, CutoverPlan, CutoverReport,
    CutoverRequest, CutoverState, MarkerStatus, inspect_marker, recover_or_verify, run_cutover,
    run_cutover_with_fault,
};
pub use error::{Result, SqliteError};
pub use exporter::{
    ExportFault, ExportLockGuard, ExportMode, ReverseExportReport, ReverseExportResult,
    acquire_export_lock, export_sqlite_to_json, export_sqlite_to_json_with_fault,
    export_sqlite_to_json_with_mode, read_export_manifest, recompute_db_export_hash,
    verify_export_tree,
};
pub use importer::{ImportReport, ImportStatus, JsonImporter};
pub use lease::{
    AUTHORITY_LEASE_FILENAME, AuthorityLeaseGuard, LeaseMode, hold_process_shared_lease,
};
pub use migrations::{Migration, builtin_migrations, current_version, migrate, migrate_with};
pub use preaccept::{
    AutofixSyncRequest, DraftAttemptOutcome, DraftAttemptRequest, PostprocessApplyOutcome,
    PostprocessApplyRequest, PreacceptFault, PreacceptOutboxKind, PreacceptOutboxRow,
    PreacceptOutboxStatus, PreacceptRecoverySnapshot, RegenerateAttemptRequest,
    SqlitePreacceptRepository,
};
pub use publication::{PublishFault, PublishOutcome, PublishRequest, SqliteChronicleRepository};
pub use readiness::{
    BackupCheckpoint, ExportSnapshot, SourceManifestReport, create_backup_checkpoint,
    export_readonly_snapshot, validate_source_manifest,
};
pub use rollback::{
    RollbackFault, RollbackReport, RollbackRequest, run_rollback, run_rollback_with_fault,
    verify_rollback_export,
};
pub use unit_of_work::UnitOfWork;

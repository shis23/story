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

pub mod connection;
pub mod contract;
pub mod error;
pub mod importer;
pub mod migrations;
pub mod production;
pub mod publication;
pub mod readiness;
pub mod unit_of_work;

pub use connection::Database;
pub use error::{Result, SqliteError};
pub use importer::{ImportReport, ImportStatus, JsonImporter};
pub use migrations::{Migration, builtin_migrations, current_version, migrate, migrate_with};
pub use publication::{PublishFault, PublishOutcome, PublishRequest, SqliteChronicleRepository};
pub use readiness::{
    BackupCheckpoint, ExportSnapshot, SourceManifestReport, create_backup_checkpoint,
    export_readonly_snapshot, validate_source_manifest,
};
pub use unit_of_work::UnitOfWork;

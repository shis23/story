//! 命令模块的宿主无关部分（DTO、命令本体之外的辅助函数）。
//!
//! `crates/tauri-app/src/commands/<同名>.rs` 只保留 `#[tauri::command]` 外壳，
//! 并 `pub use` 本模块，原有调用路径不变。

pub mod bundle_runtime;
pub mod campaigns;
pub mod cards;
pub mod characters;
pub mod connections;
pub mod conversations;
pub mod diagnostics;
pub mod import_export;
pub mod memory;
pub mod meta;
pub mod meta_typed;
pub mod plugins;
pub mod presets;
pub mod turns;
pub mod variables;
pub mod world_info;
pub mod writing;
pub mod writing_regenerate;

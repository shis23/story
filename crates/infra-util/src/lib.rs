//! 基础设施工具：原子持久化 + SecretRef 存储。
//!
//! 本 crate 汇总了原本在各 store / runtime 中逐字重复的横切逻辑：
//! - `atomic_write` / `atomic_write_json`：先写 `.tmp` 再 `rename`，崩溃时
//!   不会损坏目标文件（历史 bug：多处裸 `fs::write` 崩溃后 `from_str` 失败，
//!   被 `unwrap_or_default()` 静默清空用户数据）。
//! - `secret_store`：把 API key 写入系统凭据库，持久化文件只保存 SecretRef。

pub mod secret_store;
pub mod write_fence;

use std::io;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::Serialize;

/// 原子写入字节流到 `path`：先写 `path.tmp`（fsync），再 `rename` 到 `path`。
///
/// 若父目录不存在会自动创建。V4 硬化（2026-07-27）：
/// - temp 文件写完后 `sync_all`，掉电时 rename 后的目标不会是空洞文件；
/// - `rename` 失败带退避重试（Windows 上 AV/索引器短暂占用目标是常态），
///   重试耗尽后返回硬错误并**保留 .tmp**（最新数据在 .tmp 里可恢复）——
///   不再回退直接写 `path`（直写崩溃会撕裂主文件，比丢一次写危险得多）；
/// - Unix 上 rename 成功后 fsync 父目录，保证 rename 本身落盘。
/// - `write_fence` 冻结的路径拒绝写入（损坏未确认前不许固化空态）。
///
/// 这是项目所有 store 的持久化统一入口，取代历史上散落各处的裸 `fs::write`。
pub fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if write_fence::is_frozen(path) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!(
                "write fence active for {}: 存储文件损坏待用户确认，拒绝写入以免固化空态",
                path.display()
            ),
        ));
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // 用完整路径拼接 .tmp 后缀，避免 with_extension 在多扩展名文件上碰撞
    // （例如 data.json 和 data.json.bak 都会变成 data.tmp）
    let tmp_path = PathBuf::from(format!("{}.tmp", path.display()));
    {
        let mut file = std::fs::File::create(&tmp_path)?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }
    let mut last_err = None;
    for attempt in 0u32..3 {
        match std::fs::rename(&tmp_path, path) {
            Ok(()) => {
                #[cfg(unix)]
                if let Some(parent) = path.parent()
                    && let Ok(dir) = std::fs::File::open(parent)
                {
                    let _ = dir.sync_all();
                }
                return Ok(());
            }
            Err(e) => {
                tracing::warn!(
                    "rename {} -> {} 失败（第 {} 次）: {e}",
                    tmp_path.display(),
                    path.display(),
                    attempt + 1
                );
                last_err = Some(e);
                if attempt < 2 {
                    std::thread::sleep(std::time::Duration::from_millis(20 << attempt));
                }
            }
        }
    }
    // 重试耗尽：保留 .tmp（含最新数据，加载器可从中恢复），返回硬错误。
    let e = last_err.unwrap_or_else(|| io::Error::other("rename failed"));
    tracing::error!(
        "rename 重试耗尽，保留 {} 供恢复（不回退直写，避免撕裂主文件）: {e}",
        tmp_path.display()
    );
    Err(e)
}

/// 把可序列化数据以 pretty JSON 原子写入 `path`。
///
/// 序列化或写入失败时返回 `Err`（调用方应记录日志），但不会让目标文件
/// 处于半写状态——因为先写 `.tmp` 再 rename，目标文件要么是旧内容要么是新内容。
///
/// `T: ?Sized` 允许传 `&[U]`、`&str`、`&HashMap<..>` 等动态大小类型。
pub fn atomic_write_json<T: Serialize + ?Sized>(path: &Path, value: &T) -> io::Result<()> {
    let json = serde_json::to_vec_pretty(value)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("序列化失败: {e}")))?;
    atomic_write(path, &json)
}

/// 把 JSON 字符串原子写入 `path`（给已有 JSON 字符串的场景用，避免重复序列化）。
pub fn atomic_write_json_str(path: &Path, json: &str) -> io::Result<()> {
    atomic_write(path, json.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atomic_write_roundtrip() {
        let dir = std::env::temp_dir().join(format!("sf_util_{}", uuid_like()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("test.json");
        atomic_write(&path, b"{\"x\":1}").unwrap();
        let read = std::fs::read_to_string(&path).unwrap();
        assert_eq!(read, "{\"x\":1}");
        // 覆盖写
        atomic_write(&path, b"{\"y\":2}").unwrap();
        let read = std::fs::read_to_string(&path).unwrap();
        assert_eq!(read, "{\"y\":2}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn atomic_write_creates_parent_dir() {
        let dir = std::env::temp_dir().join(format!("sf_util_nested_{}", uuid_like()));
        let path = dir.join("sub").join("deep.json");
        atomic_write_json_str(&path, "{\"ok\":true}").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{\"ok\":true}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn atomic_write_json_pretty() {
        let dir = std::env::temp_dir().join(format!("sf_util_json_{}", uuid_like()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("data.json");
        let val = serde_json::json!({"a": 1, "b": [2, 3]});
        atomic_write_json(&path, &val).unwrap();
        let read = std::fs::read_to_string(&path).unwrap();
        assert!(read.contains("\"a\": 1"));
        assert!(read.contains("\"b\""));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 简单的唯一 id 生成（测试用，避免引入 uuid 依赖到 infra-util）。
    fn uuid_like() -> String {
        use std::time::{SystemTime, UNIX_EPOCH};
        format!(
            "{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        )
    }
}

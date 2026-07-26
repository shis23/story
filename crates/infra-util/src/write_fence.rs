//! 写栅栏（V4）：存储文件被判定损坏且无法自动恢复时，冻结该路径的写入，
//! 直到用户在恢复引导里明确确认（从空白开始 / 手动修复后解除）。
//!
//! 动机：历史行为是损坏加载静默返回空集，下一次保存把空集写回主文件——
//! 用户数据从"可抢救的损坏文件"变成"干净的空文件"，永久丢失。
//! 栅栏保证：损坏未确认前，`atomic_write` 对该路径直接拒绝。
//!
//! 纯 std 全局状态，无 Tauri/UI 依赖；事件详情（错误、备份路径）由上层
//! （tauri-app `storage_health`）登记并服务前端。

use std::collections::HashSet;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

static ANY_FROZEN: AtomicBool = AtomicBool::new(false);
static FROZEN: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();

fn frozen_set() -> &'static Mutex<HashSet<String>> {
    FROZEN.get_or_init(|| Mutex::new(HashSet::new()))
}

/// 路径归一化：用字符串形态做 key（同一路径必须来自同一 store 常量拼接，
/// 不做符号链接解析——store 层总是用同一 PathBuf 构造方式）。
fn key(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// 冻结路径：后续 `atomic_write` 对它返回 PermissionDenied。
pub fn freeze(path: &Path) {
    let mut set = frozen_set().lock().unwrap_or_else(|p| p.into_inner());
    set.insert(key(path));
    ANY_FROZEN.store(true, Ordering::SeqCst);
}

/// 解冻路径（用户确认后调用）。返回是否原本处于冻结。
pub fn unfreeze(path: &Path) -> bool {
    let mut set = frozen_set().lock().unwrap_or_else(|p| p.into_inner());
    let removed = set.remove(&key(path));
    if set.is_empty() {
        ANY_FROZEN.store(false, Ordering::SeqCst);
    }
    removed
}

/// 查询路径是否被冻结。热路径上无冻结时只读一个原子布尔。
pub fn is_frozen(path: &Path) -> bool {
    if !ANY_FROZEN.load(Ordering::SeqCst) {
        return false;
    }
    frozen_set()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .contains(&key(path))
}

/// 当前全部冻结路径（诊断/UI 用）。
pub fn frozen_paths() -> Vec<String> {
    if !ANY_FROZEN.load(Ordering::SeqCst) {
        return vec![];
    }
    frozen_set()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .iter()
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn freeze_blocks_atomic_write_until_unfreeze() {
        let dir = std::env::temp_dir().join(format!(
            "sf_fence_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path: PathBuf = dir.join("frozen.json");

        assert!(!is_frozen(&path));
        freeze(&path);
        assert!(is_frozen(&path));
        assert!(frozen_paths().iter().any(|p| p.contains("frozen.json")));

        let err = crate::atomic_write(&path, b"{}").unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::PermissionDenied);
        assert!(!path.exists(), "冻结期间不得产生任何写入");

        assert!(unfreeze(&path));
        assert!(!is_frozen(&path));
        crate::atomic_write(&path, b"{\"ok\":1}").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{\"ok\":1}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unrelated_paths_stay_writable_while_one_is_frozen() {
        let dir = std::env::temp_dir().join(format!(
            "sf_fence_iso_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let frozen = dir.join("a.json");
        let free = dir.join("b.json");

        freeze(&frozen);
        crate::atomic_write(&free, b"{}").unwrap();
        assert!(free.exists());
        assert!(unfreeze(&frozen));

        let _ = std::fs::remove_dir_all(&dir);
    }
}

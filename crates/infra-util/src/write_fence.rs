//! 写栅栏（V4）：存储文件被判定损坏且无法自动恢复时，冻结该路径的写入，
//! 直到用户在恢复引导里明确确认（从空白开始 / 手动修复后解除）。
//!
//! 动机：历史行为是损坏加载静默返回空集，下一次保存把空集写回主文件——
//! 用户数据从"可抢救的损坏文件"变成"干净的空文件"，永久丢失。
//! 栅栏保证：损坏未确认前，`atomic_write` 对该路径直接拒绝。
//!
//! 纯 std 全局状态，无 Tauri/UI 依赖；事件详情（错误、备份路径）由上层
//! （tauri-app `storage_health`）登记并服务前端。

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

static ANY_FROZEN: AtomicBool = AtomicBool::new(false);
static FROZEN: OnceLock<Mutex<HashMap<String, FrozenEntry>>> = OnceLock::new();

/// 一个被冻结（隔离）的路径及其原因快照（只读诊断用）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrozenEntry {
    /// 被隔离的文件路径（字符串形态，与冻结时同一构造方式）。
    pub path: String,
    /// 为什么冻结（含错误摘要，用于健康报告/支持排查）。
    pub reason: String,
    /// 冻结时刻（Unix 秒；仅用于展示先后顺序）。
    pub frozen_at_epoch_secs: u64,
}

impl FrozenEntry {
    /// 面向用户/支持的一句话说明：哪份文件被隔离、下一步该做什么。
    ///
    /// Lead 裁定（D-03 追加可观测性）：冻结必须可观测、可恢复——只报错不给
    /// 出路是不可接受的终态；应用内一键解除属产品决策（N-03，本轮不做），
    /// 所以这里的文案指向"备份/移走文件后重启"这条人工路径。
    pub fn summary(&self) -> String {
        format!(
            "存储文件已被隔离（写入冻结）：{}；原因：{}。\
             处理建议：先备份/移走该文件（或确认可从空白重建），然后重启应用；\
             本次启动期间对该文件的所有写入都会被拒绝，以免覆盖可抢救的数据。",
            self.path, self.reason
        )
    }
}

/// 冻结路径时的默认原因（无额外上下文的调用方）。
const DEFAULT_FREEZE_REASON: &str = "加载失败且无法自动恢复";

fn now_epoch_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn frozen_map() -> &'static Mutex<HashMap<String, FrozenEntry>> {
    FROZEN.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 路径归一化：用字符串形态做 key（同一路径必须来自同一 store 常量拼接，
/// 不做符号链接解析——store 层总是用同一 PathBuf 构造方式）。
fn key(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// 冻结路径：后续 `atomic_write` 对它返回 PermissionDenied。
pub fn freeze(path: &Path) {
    freeze_with_reason(path, DEFAULT_FREEZE_REASON);
}

/// 冻结路径并记录原因（推荐：让健康报告能说清"哪份文件、为什么、怎么办"）。
pub fn freeze_with_reason(path: &Path, reason: &str) {
    let mut map = frozen_map().lock().unwrap_or_else(|p| p.into_inner());
    map.insert(
        key(path),
        FrozenEntry {
            path: key(path),
            reason: reason.to_string(),
            frozen_at_epoch_secs: now_epoch_secs(),
        },
    );
    ANY_FROZEN.store(true, Ordering::SeqCst);
}

/// 解冻路径（用户确认后调用）。返回是否原本处于冻结。
pub fn unfreeze(path: &Path) -> bool {
    let mut map = frozen_map().lock().unwrap_or_else(|p| p.into_inner());
    let removed = map.remove(&key(path)).is_some();
    if map.is_empty() {
        ANY_FROZEN.store(false, Ordering::SeqCst);
    }
    removed
}

/// 查询路径是否被冻结。热路径上无冻结时只读一个原子布尔。
pub fn is_frozen(path: &Path) -> bool {
    if !ANY_FROZEN.load(Ordering::SeqCst) {
        return false;
    }
    frozen_map()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .contains_key(&key(path))
}

/// 该路径的冻结原因（未冻结返回 None）。只读查询。
pub fn frozen_reason(path: &Path) -> Option<String> {
    if !ANY_FROZEN.load(Ordering::SeqCst) {
        return None;
    }
    frozen_map()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .get(&key(path))
        .map(|e| e.reason.clone())
}

/// 当前全部冻结路径（诊断/UI 用）。
pub fn frozen_paths() -> Vec<String> {
    frozen_entries().into_iter().map(|e| e.path).collect()
}

/// 全部冻结条目（含原因/时间），按冻结时间升序。
pub fn frozen_entries() -> Vec<FrozenEntry> {
    if !ANY_FROZEN.load(Ordering::SeqCst) {
        return vec![];
    }
    let mut entries: Vec<FrozenEntry> = frozen_map()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .values()
        .cloned()
        .collect();
    entries.sort_by(|a, b| {
        a.frozen_at_epoch_secs
            .cmp(&b.frozen_at_epoch_secs)
            .then_with(|| a.path.cmp(&b.path))
    });
    entries
}

/// 供上层健康面（tauri-app `storage_health::record_backend_incident`）直接消费
/// 的 `(kind, detail)` 列表——调用方无需了解 write_fence 内部结构。
///
/// 域1 只提供这份只读报告；把事件推给前端的接线在 tauri-app（域4/域2 写作用域）。
pub fn storage_health_report() -> Vec<(String, String)> {
    frozen_entries()
        .into_iter()
        .map(|e| ("write_fence_frozen".to_string(), e.summary()))
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

    #[test]
    fn frozen_reason_and_health_report_explain_the_freezed_file() {
        // Lead 裁定 D-03 追加可观测性：冻结必须可观测（哪份文件、为什么、怎么办）
        let dir = std::env::temp_dir().join(format!(
            "sf_fence_health_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("vectors.json");
        let reason = "主文件与 .tmp 均无法解析（expected ident）；已备份到 vectors.json.corrupt";

        freeze_with_reason(&path, reason);

        assert_eq!(frozen_reason(&path).as_deref(), Some(reason));
        let entries = frozen_entries();
        let entry = entries
            .iter()
            .find(|e| e.path.contains("vectors.json"))
            .expect("冻结条目必须出现在诊断快照里");
        assert_eq!(entry.reason, reason);
        assert!(entry.frozen_at_epoch_secs > 0);

        // 健康报告：调用方（tauri-app storage_health）可直接消费的 (kind, detail)
        let report = storage_health_report();
        let (kind, detail) = report
            .iter()
            .find(|(_, detail)| detail.contains("vectors.json"))
            .expect("冻结状态必须能出现在健康报告里");
        assert_eq!(kind, "write_fence_frozen");
        assert!(detail.contains("已被隔离"), "{detail}");
        assert!(detail.contains(reason), "报告必须带原因：{detail}");
        assert!(
            detail.contains("处理建议"),
            "报告必须带人工处理指引：{detail}"
        );

        // 解冻后报告清空（不残留陈旧事件）
        assert!(unfreeze(&path));
        assert_eq!(frozen_reason(&path), None);
        assert!(
            !storage_health_report()
                .iter()
                .any(|(_, d)| d.contains("vectors.json"))
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn freeze_without_reason_still_reports_a_default_reason() {
        let dir = std::env::temp_dir().join(format!(
            "sf_fence_default_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("plain.json");

        freeze(&path);
        assert_eq!(frozen_reason(&path).as_deref(), Some(DEFAULT_FREEZE_REASON));
        assert!(unfreeze(&path));

        let _ = std::fs::remove_dir_all(&dir);
    }
}

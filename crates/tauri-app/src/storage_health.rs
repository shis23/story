//! 存储健康登记（V4）：JSON store 加载损坏事件的进程级登记簿。
//!
//! 数据源是 `json_store::load_json_with_tmp_backup_or_default` 的三种结果：
//! - 主文件损坏、.tmp 恢复成功 → 非阻断事件（下次保存会用好数据重写主文件）；
//! - 主文件损坏、.tmp 也不可用 → **阻断事件**：`write_fence::freeze` 该路径，
//!   前端启动时弹恢复引导，用户确认前该文件的所有写入被拒绝（不固化空态）；
//! - 主文件存在但读不出来（IO 错误）→ 同阻断事件处理。
//!
//! 前端流程：`storage_health_report` 拉事件 → 阻断事件弹引导 →
//! 用户选「从空白开始」调 `storage_health_acknowledge(path)` 解冻；
//! 选「稍后手动修复」则保持冻结（保存会报清晰错误而不是静默丢数据）。

use std::path::Path;
use std::sync::{Mutex, OnceLock};

#[derive(Debug, Clone, serde::Serialize)]
pub struct StorageIncident {
    /// 损坏的主文件绝对路径
    pub path: String,
    /// 解析/IO 错误文本
    pub error: String,
    /// `.corrupt` 备份路径（阻断事件时生成，可供手动修复）
    pub corrupt_backup: Option<String>,
    /// true = 主文件损坏但 .tmp 恢复成功（数据无损，仅提示）
    pub recovered_from_tmp: bool,
    /// true = 该路径处于写栅栏冻结中，等待用户确认
    pub blocking: bool,
    pub detected_at: String,
}

static INCIDENTS: OnceLock<Mutex<Vec<StorageIncident>>> = OnceLock::new();

fn incidents_store() -> &'static Mutex<Vec<StorageIncident>> {
    INCIDENTS.get_or_init(|| Mutex::new(Vec::new()))
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}

fn push_incident(incident: StorageIncident) {
    let mut list = incidents_store().lock().unwrap_or_else(|p| p.into_inner());
    // 同一路径去重：保留最新一条（重复加载同一损坏文件不刷屏）
    list.retain(|i| i.path != incident.path);
    list.push(incident);
}

/// 主文件损坏但 .tmp 恢复成功：登记非阻断事件（不冻结）。
pub fn record_tmp_recovered(path: &Path, error: &str) {
    push_incident(StorageIncident {
        path: path.to_string_lossy().into_owned(),
        error: error.to_string(),
        corrupt_backup: None,
        recovered_from_tmp: true,
        blocking: false,
        detected_at: now(),
    });
}

/// 主文件损坏且无法恢复：冻结写入 + 登记阻断事件。
pub fn record_unrecoverable(path: &Path, error: &str, corrupt_backup: Option<&Path>) {
    storyforge_infra_util::write_fence::freeze(path);
    push_incident(StorageIncident {
        path: path.to_string_lossy().into_owned(),
        error: error.to_string(),
        corrupt_backup: corrupt_backup.map(|p| p.to_string_lossy().into_owned()),
        recovered_from_tmp: false,
        blocking: true,
        detected_at: now(),
    });
}

/// 全部事件快照（阻断在前，前端直接展示）。
pub fn incidents() -> Vec<StorageIncident> {
    let mut list = incidents_store()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .clone();
    list.sort_by_key(|i| std::cmp::Reverse(i.blocking));
    list
}

/// 用户确认某路径「从空白开始」：解冻并把事件降级为非阻断。
/// 返回是否确有该路径的阻断事件。
pub fn acknowledge(path: &str) -> bool {
    let unfrozen = storyforge_infra_util::write_fence::unfreeze(Path::new(path));
    let mut list = incidents_store().lock().unwrap_or_else(|p| p.into_inner());
    let mut found = false;
    for incident in list.iter_mut() {
        if incident.path == path && incident.blocking {
            incident.blocking = false;
            found = true;
        }
    }
    found || unfrozen
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unrecoverable_freezes_then_acknowledge_unfreezes() {
        let dir = std::env::temp_dir().join(format!("sf_health_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("campaigns.json");

        record_unrecoverable(&path, "expected ident", None);
        assert!(storyforge_infra_util::write_fence::is_frozen(&path));
        let listed = incidents();
        let mine = listed
            .iter()
            .find(|i| i.path == path.to_string_lossy())
            .expect("incident recorded");
        assert!(mine.blocking);

        assert!(acknowledge(&path.to_string_lossy()));
        assert!(!storyforge_infra_util::write_fence::is_frozen(&path));
        let after = incidents();
        let mine = after
            .iter()
            .find(|i| i.path == path.to_string_lossy())
            .unwrap();
        assert!(!mine.blocking);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn tmp_recovered_incident_is_informational() {
        let dir = std::env::temp_dir().join(format!("sf_health_info_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("presets.json");

        record_tmp_recovered(&path, "trailing comma");
        assert!(!storyforge_infra_util::write_fence::is_frozen(&path));
        let listed = incidents();
        let mine = listed
            .iter()
            .find(|i| i.path == path.to_string_lossy())
            .unwrap();
        assert!(mine.recovered_from_tmp);
        assert!(!mine.blocking);

        let _ = std::fs::remove_dir_all(&dir);
    }
}

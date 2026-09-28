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

/// 后端级事件（S-01/S-17）：SQLite 权威侧的静默风险必须在健康面可见。
///
/// 数据源：cutover/导入阶段的真实非零跳过计数、stale marker 自愈、SQLite
/// 运行期失联等。`path` 用后端标识（如 `sqlite:<db 文件名>`）而非用户目录，
/// 避免把绝对路径暴露到前端。
pub fn record_backend_incident(kind: &str, detail: &str) {
    let id = format!("backend:{kind}");
    push_incident(StorageIncident {
        path: id,
        error: detail.to_string(),
        corrupt_backup: None,
        recovered_from_tmp: false,
        // 非阻断：数据仍在（源 JSON 未删），但必须让用户/支持看到。
        blocking: false,
        detected_at: now(),
    });
}

/// 把域1 `write_fence` 的冻结状态并入健康报告（启动时调用一次）。
///
/// 与 `json_store` 登记的关系（去重策略）：
/// - 已被 `record_unrecoverable` 登记为**阻断事件**的路径跳过——那条更完整
///   （带 `.corrupt` 备份、`recovered_from_tmp` 语义、用户确认流程），且
///   `push_incident` 按 path 去重，再登记会把它覆盖成信息更少的一条；
/// - 其余冻结条目（例如向量库 `vectors.json`：写入失败 → `write_fence::freeze`
///   但没有经过 json_store 加载路径）按**真实文件路径**登记一条阻断事件，
///   这样前端的 `storage_health_acknowledge(path)` 能直接解冻该文件；
/// - `error` 前缀写入域1 约定的 kind 字符串 `write_fence_frozen`，供诊断/支持
///   按关键字检索（`StorageIncident` 没有独立 kind 字段，不为此改前端 DTO）。
///
/// 幂等：同一路径重复调用只保留最新一条（`push_incident` 的去重语义）。
pub fn record_write_fence_state() {
    let already_registered: Vec<String> = incidents_store()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .iter()
        .filter(|i| i.blocking)
        .map(|i| i.path.clone())
        .collect();

    for entry in storyforge_infra_util::write_fence::frozen_entries() {
        if already_registered.iter().any(|path| path == &entry.path) {
            continue;
        }
        push_incident(StorageIncident {
            path: entry.path.clone(),
            error: format!("write_fence_frozen: {}", entry.summary()),
            corrupt_backup: None,
            recovered_from_tmp: false,
            blocking: true,
            detected_at: now(),
        });
    }
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
    fn frozen_store_surfaces_as_write_fence_frozen_incident() {
        // 跨域接线（域1 D-03）：向量库等被 write_fence 冻结的路径必须出现在
        // 既有健康报告里，否则"写入持续 PermissionDenied"在 UI 上无声。
        let dir = std::env::temp_dir().join(format!("sf_health_fence_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("vectors.json");
        storyforge_infra_util::write_fence::freeze_with_reason(&path, "向量库写入失败：目标不可写");

        record_write_fence_state();
        let listed = incidents();
        let mine = listed
            .iter()
            .find(|i| i.path == path.to_string_lossy())
            .expect("冻结路径必须登记为健康事件");
        assert!(
            mine.error.contains("write_fence_frozen"),
            "事件必须带域1 约定的 kind 关键字，got: {}",
            mine.error
        );
        assert!(mine.blocking, "冻结必须是阻断事件（需用户确认或人工修复）");
        assert!(
            mine.error.contains("向量库写入失败"),
            "事件必须带上冻结原因，got: {}",
            mine.error
        );

        // 幂等：同一路径重复扫描不刷屏。
        record_write_fence_state();
        assert_eq!(
            incidents()
                .iter()
                .filter(|i| i.path == path.to_string_lossy())
                .count(),
            1,
            "同一路径不得重复登记"
        );

        // 前端确认（解冻）后事件降级；解冻发生在真实路径上。
        assert!(acknowledge(&path.to_string_lossy()));
        assert!(!storyforge_infra_util::write_fence::is_frozen(&path));
        let _ = storyforge_infra_util::write_fence::unfreeze(&path);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_fence_sweep_keeps_the_richer_json_store_incident() {
        // json_store 已登记的损坏路径不被 sweep 覆盖：那条带 .corrupt 备份与
        // 原始解析错误，信息更完整（互补而非重复）。
        let dir = std::env::temp_dir().join(format!("sf_health_dedup_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("campaigns.json");
        record_unrecoverable(&path, "expected ident at line 3", None);

        record_write_fence_state();
        let matching: Vec<_> = incidents()
            .into_iter()
            .filter(|i| i.path == path.to_string_lossy())
            .collect();
        assert_eq!(matching.len(), 1, "不得重复登记同一路径");
        assert!(matching[0].blocking);
        assert!(
            matching[0].error.contains("expected ident"),
            "必须保留 json_store 的原始错误，got: {}",
            matching[0].error
        );
        assert!(
            !matching[0].error.contains("write_fence_frozen"),
            "已被 json_store 登记的路径不应被 sweep 重写"
        );

        let _ = storyforge_infra_util::write_fence::unfreeze(&path);
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

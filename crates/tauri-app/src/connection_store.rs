/// LLM 连接存储（JSON 文件持久化 + 系统凭据库存 key）
///
/// 存储位置：data/connections.json
/// 结构：{ active_id: Option<String>, connections: Vec<StoredConnection> }
///
/// 注意：`connection.api_key` 在磁盘上只保存 SecretRef；运行时读取时会解析为真实 key。
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::storage::json_store;
use storyforge_domain::llm::LlmConnection;
use storyforge_infra_util::secret_store::{
    SecretStore, SystemSecretStore, is_secret_ref, make_secret_ref, resolve_secret_value,
};

const LLM_SECRET_KIND: &str = "llm-connection";

/// 已存储的连接；磁盘上的 `connection.api_key` 是 SecretRef。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredConnection {
    pub id: String,
    pub connection: LlmConnection,
    pub created_at: String,
    /// 最后使用时间（设为活跃时更新）
    pub last_used_at: Option<String>,
}

/// 文件顶层结构
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ConnectionsFile {
    /// 当前活跃连接 ID（None = 无活跃）
    #[serde(default)]
    pub active_id: Option<String>,
    pub connections: Vec<StoredConnection>,
}

/// 连接存储
pub struct ConnectionStore {
    path: PathBuf,
    secret_store: Arc<dyn SecretStore>,
    inner: Mutex<ConnectionsFile>,
}

impl ConnectionStore {
    /// 初始化（从文件加载或新建）
    pub fn new(app_data_dir: &Path) -> Self {
        Self::new_with_secret_store(app_data_dir, Arc::new(SystemSecretStore::default()))
    }

    pub fn new_with_secret_store(app_data_dir: &Path, secret_store: Arc<dyn SecretStore>) -> Self {
        let path = app_data_dir.join("connections.json");
        let file: ConnectionsFile = json_store::load_json_with_tmp_backup_or_default(
            &path,
            |e| tracing::warn!("连接配置 JSON 解析失败({e})，尝试 .tmp 备份"),
            |path, e| {
                tracing::error!(
                    "连接配置 JSON 主文件和 .tmp 备份均损坏，文件: {}, 错误: {}. 已保存 .corrupt 备份",
                    path.display(),
                    e
                )
            },
        );
        let store = Self {
            path,
            secret_store,
            inner: Mutex::new(file),
        };
        if let Err(e) = store.migrate_plaintext_api_keys() {
            tracing::warn!("迁移连接 API key 到系统凭据库失败，保留旧文件: {e}");
        }
        store
    }

    /// 保存连接（新建或更新）
    pub fn save(&self, connection: LlmConnection) -> Result<StoredConnection, String> {
        let mut file = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        // Gate 8 审查 P2-B7 同款快照：persist 失败必须回滚内存，避免
        // 「内存已改/磁盘未变」的分裂态。覆盖更新与新增两条分支。
        let snapshot = file.clone();

        // 若已存在同 id，更新；否则新增
        let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
        let conn_id = connection.id.as_str().to_string();
        let mut stored_connection = connection.clone();
        stored_connection.api_key = self.secure_api_key(&conn_id, &connection.api_key)?;

        if let Some(existing) = file.connections.iter_mut().find(|c| c.id == conn_id) {
            existing.connection = stored_connection.clone();
            existing.last_used_at = Some(now.clone());
            let updated = existing.clone();
            if let Err(e) = self.persist(&file) {
                *file = snapshot;
                return Err(e);
            }
            return Ok(updated);
        }

        let stored = StoredConnection {
            id: conn_id,
            connection: stored_connection,
            created_at: now,
            last_used_at: None,
        };
        file.connections.push(stored.clone());
        if let Err(e) = self.persist(&file) {
            *file = snapshot;
            return Err(e);
        }
        Ok(stored)
    }

    /// 更新已有连接。`connection.api_key` 为空时保留原 SecretRef / 明文 key。
    ///
    /// 返回解析后的运行时连接（含真实 api_key），供活跃连接热刷新 client。
    pub fn update_existing(
        &self,
        id: &str,
        mut connection: LlmConnection,
    ) -> Result<LlmConnection, String> {
        let existing = self.get(id).ok_or_else(|| format!("连接不存在: {id}"))?;
        connection.id = storyforge_domain::Id::from_str(id);
        if connection.api_key.trim().is_empty() {
            // 保留磁盘上的 SecretRef，避免空串把 key 清掉
            connection.api_key = existing.connection.api_key;
        }
        self.save(connection)?;
        self.resolved(id)?
            .ok_or_else(|| format!("连接不存在: {id}"))
    }

    /// 读取并解析为运行时连接（SecretRef → 真实 key）。不含 key 的展示请用 `get`。
    pub fn resolved(&self, id: &str) -> Result<Option<LlmConnection>, String> {
        match self.get(id) {
            Some(stored) => Ok(Some(self.resolve_connection(&stored.connection)?)),
            None => Ok(None),
        }
    }

    /// 列出所有连接
    pub fn list(&self) -> Vec<StoredConnection> {
        self.inner
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .connections
            .clone()
    }

    /// 获取单个连接
    pub fn get(&self, id: &str) -> Option<StoredConnection> {
        self.inner
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .connections
            .iter()
            .find(|c| c.id == id)
            .cloned()
    }

    /// 删除连接（若是活跃的，同时清除 active_id）
    pub fn delete(&self, id: &str) -> Result<bool, String> {
        let mut file = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        // 先快照修改前状态——persist 失败时回滚内存，避免「UI 已删但重启
        // 复活」的分裂态（Gate 8 审查 P2-B7，turn_store 同款模式）。快照
        // 必须在 retain 之前取，否则回滚后连接仍是被删状态。
        let snapshot = file.clone();
        let before = file.connections.len();
        let mut removed_secret_refs = Vec::new();
        file.connections.retain(|c| {
            if c.id == id {
                if is_secret_ref(&c.connection.api_key) {
                    removed_secret_refs.push(c.connection.api_key.clone());
                }
                false
            } else {
                true
            }
        });
        if file.connections.len() < before {
            // 若删除的是活跃连接，清除 active_id
            if file.active_id.as_deref() == Some(id) {
                file.active_id = None;
            }
            if let Err(e) = self.persist(&file) {
                *file = snapshot;
                return Err(e);
            }
            for secret_ref in removed_secret_refs {
                if let Err(e) = self.secret_store.delete_secret(&secret_ref) {
                    tracing::warn!("删除连接 SecretRef 失败 {secret_ref}: {e}");
                }
            }
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// 设置活跃连接 ID（会校验该 id 存在，并更新 last_used_at）
    ///
    /// 返回对应的 LlmConnection（供调用方构造 client）。
    pub fn set_active(&self, id: &str) -> Result<Option<LlmConnection>, String> {
        let mut file = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        // 快照：persist 失败回滚 last_used_at / active_id（Gate 8 审查
        // P2-B7）。取在一切内存修改之前，回滚语义才完整。
        let snapshot = file.clone();
        let conn = {
            let stored = match file.connections.iter_mut().find(|c| c.id == id) {
                Some(s) => s,
                None => return Ok(None),
            };
            let conn = self.resolve_connection(&stored.connection)?;
            stored.last_used_at =
                Some(chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string());
            conn
        };
        file.active_id = Some(id.to_string());
        if let Err(e) = self.persist(&file) {
            *file = snapshot;
            return Err(e);
        }
        Ok(Some(conn))
    }

    /// 获取活跃连接（若 active_id 存在且对应连接存在）
    pub fn active_connection(&self) -> Option<LlmConnection> {
        let file = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        let active_id = file.active_id.as_ref()?;
        let conn = file
            .connections
            .iter()
            .find(|c| &c.id == active_id)
            .map(|c| c.connection.clone())?;
        match self.resolve_connection(&conn) {
            Ok(conn) => Some(conn),
            Err(e) => {
                tracing::warn!("读取活跃连接 SecretRef 失败: {e}");
                None
            }
        }
    }

    /// Re-run plaintext→SecretRef migration. Idempotent (already-migrated
    /// SecretRefs are skipped). Used on Android after ndk-context finishes
    /// its async initialization: the first migration attempt ran during
    /// `ConnectionStore::new` (synchronously in the setup hook, before
    /// ndk-context was ready) and fail-closed to plaintext; this retries it
    /// against the now-available Keystore. Errors are non-fatal — the caller
    /// logs a warning and keeps plaintext (resolve_secret_value passes it
    /// through), so the app keeps working either way.
    ///
    /// The only call site is Android-gated (lib.rs ndk-context retry task);
    /// desktop builds never invoke it.
    #[cfg_attr(not(target_os = "android"), expect(dead_code))]
    pub fn retry_migration(&self) {
        if let Err(e) = self.migrate_plaintext_api_keys() {
            tracing::warn!("重试迁移连接 API key 到系统凭据库失败，保留旧文件: {e}");
        }
    }

    fn migrate_plaintext_api_keys(&self) -> Result<(), String> {
        let mut file = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        let mut changed = false;
        for stored in &mut file.connections {
            let api_key = stored.connection.api_key.clone();
            if api_key.is_empty() || is_secret_ref(&api_key) {
                continue;
            }
            stored.connection.api_key = self.secure_api_key(&stored.id, &api_key)?;
            changed = true;
        }
        if changed {
            self.persist(&file)?;
        }
        Ok(())
    }

    fn secure_api_key(&self, conn_id: &str, api_key: &str) -> Result<String, String> {
        if api_key.is_empty() || is_secret_ref(api_key) {
            return Ok(api_key.to_string());
        }
        let secret_ref = make_secret_ref(LLM_SECRET_KIND, conn_id);
        self.secret_store.put_secret(&secret_ref, api_key)?;
        Ok(secret_ref)
    }

    fn resolve_connection(&self, connection: &LlmConnection) -> Result<LlmConnection, String> {
        let mut conn = connection.clone();
        conn.api_key = resolve_secret_value(&conn.api_key, self.secret_store.as_ref())?;
        Ok(conn)
    }

    fn persist(&self, file: &ConnectionsFile) -> Result<(), String> {
        storyforge_infra_util::atomic_write_json(&self.path, file).map_err(|e| {
            let msg = format!("持久化连接配置失败: {e}");
            tracing::error!("{msg}");
            msg
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use storyforge_domain::llm::{LlmProtocol, SamplingParams, ToolMode};

    #[derive(Default)]
    struct MemorySecretStore {
        secrets: Mutex<HashMap<String, String>>,
        deleted: Mutex<Vec<String>>,
    }

    impl SecretStore for MemorySecretStore {
        fn put_secret(&self, secret_ref: &str, secret: &str) -> Result<(), String> {
            self.secrets
                .lock()
                .unwrap()
                .insert(secret_ref.to_string(), secret.to_string());
            Ok(())
        }

        fn get_secret(&self, secret_ref: &str) -> Result<String, String> {
            self.secrets
                .lock()
                .unwrap()
                .get(secret_ref)
                .cloned()
                .ok_or_else(|| format!("missing secret {secret_ref}"))
        }

        fn delete_secret(&self, secret_ref: &str) -> Result<(), String> {
            self.secrets.lock().unwrap().remove(secret_ref);
            self.deleted.lock().unwrap().push(secret_ref.to_string());
            Ok(())
        }
    }

    fn make_conn(name: &str) -> LlmConnection {
        LlmConnection {
            id: storyforge_domain::Id::from_str(name),
            name: name.into(),
            base_url: "https://api.deepseek.com".into(),
            api_key: "sk-test".into(),
            model: "deepseek-chat".into(),
            protocol: LlmProtocol::OpenAi,
            params: SamplingParams::default(),
            tool_mode: ToolMode::Native,
        }
    }

    fn temp_store() -> ConnectionStore {
        temp_store_with_secret_store().0
    }

    fn temp_store_with_secret_store() -> (ConnectionStore, Arc<MemorySecretStore>, PathBuf) {
        let dir =
            std::env::temp_dir().join(format!("storyforge_test_conn_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let secret_store = Arc::new(MemorySecretStore::default());
        (
            ConnectionStore::new_with_secret_store(&dir, secret_store.clone()),
            secret_store,
            dir,
        )
    }

    #[test]
    fn test_save_list_delete() {
        let store = temp_store();
        store.save(make_conn("deepseek-1")).unwrap();
        store.save(make_conn("deepseek-2")).unwrap();

        assert_eq!(store.list().len(), 2);
        assert!(store.get("deepseek-1").is_some());
        assert!(store.get("nonexistent").is_none());

        assert!(store.delete("deepseek-1").unwrap());
        assert_eq!(store.list().len(), 1);
        assert!(!store.delete("nonexistent").unwrap());
    }

    #[test]
    fn test_active_connection_set_and_clear() {
        let store = temp_store();
        store.save(make_conn("c1")).unwrap();
        store.save(make_conn("c2")).unwrap();

        assert!(store.active_connection().is_none());

        // 设 c1 为活跃
        let conn = store.set_active("c1").unwrap();
        assert!(conn.is_some());
        assert_eq!(store.active_connection().unwrap().id.as_str(), "c1");

        // 删除活跃的 c1，active_id 应清除
        assert!(store.delete("c1").unwrap());
        assert!(store.active_connection().is_none());
    }

    #[test]
    fn test_persistence_across_instances() {
        let dir = std::env::temp_dir().join(format!(
            "storyforge_test_conn_persist_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let secret_store = Arc::new(MemorySecretStore::default());

        {
            let store = ConnectionStore::new_with_secret_store(&dir, secret_store.clone());
            store.save(make_conn("persist-1")).unwrap();
            store.set_active("persist-1").unwrap();
        }

        // 新实例从同一文件加载
        let store2 = ConnectionStore::new_with_secret_store(&dir, secret_store);
        assert_eq!(store2.list().len(), 1);
        assert_eq!(
            store2.active_connection().unwrap().api_key,
            "sk-test",
            "运行时应能从 SecretRef 解析真实 key"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_save_stores_secret_ref_not_plaintext() {
        let (store, secret_store, dir) = temp_store_with_secret_store();
        store.save(make_conn("deepseek-secure")).unwrap();

        let raw = std::fs::read_to_string(dir.join("connections.json")).unwrap();
        assert!(!raw.contains("sk-test"));
        assert!(raw.contains(storyforge_infra_util::secret_store::SECRET_REF_PREFIX));

        let stored = store.get("deepseek-secure").unwrap();
        assert!(is_secret_ref(&stored.connection.api_key));
        assert_eq!(
            secret_store
                .get_secret(&stored.connection.api_key)
                .expect("secret stored"),
            "sk-test"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_set_active_resolves_secret_ref() {
        let (store, _secret_store, dir) = temp_store_with_secret_store();
        store.save(make_conn("deepseek-active")).unwrap();

        let conn = store.set_active("deepseek-active").unwrap().unwrap();
        assert_eq!(conn.api_key, "sk-test");
        assert_eq!(store.active_connection().unwrap().api_key, "sk-test");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn persist_failure_rolls_back_memory_for_save_delete_set_active() {
        // Gate 8 审查 P2-B7 判别测试：persist 失败时 save（更新/新增）、
        // delete、set_active 的内存态必须全部回滚——旧实现（快照在 retain
        // 之后取、save 分支无快照）会在内存/磁盘间留下分裂态，本测试在
        // 故障注入下 RED。
        let (store, _secret_store, dir) = temp_store_with_secret_store();
        store.save(make_conn("c1")).unwrap();
        store.save(make_conn("c2")).unwrap();
        store.set_active("c2").unwrap();

        // 故障注入：把 connections.json 原地替换为目录，后续 persist 必失败。
        let path = store.path.clone();
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir_all(&path).unwrap();

        // 1) save 更新分支失败 → 内存保留旧值（last_used_at 不变）。
        let before_update = store.get("c1").unwrap().last_used_at.clone();
        assert!(store.save(make_conn("c1")).is_err());
        assert_eq!(store.get("c1").unwrap().last_used_at, before_update);
        assert_eq!(store.list().len(), 2);

        // 2) save 新增分支失败 → 不新增（旧实现 push 后 persist 失败会残留）。
        assert!(store.save(make_conn("c3")).is_err());
        assert_eq!(store.list().len(), 2);
        assert!(store.get("c3").is_none());

        // 3) set_active 失败 → active 仍是 c2。
        assert!(store.set_active("c1").is_err());
        assert_eq!(store.active_connection().unwrap().id.as_str(), "c2");

        // 4) delete 失败 → c2 仍在内存（旧实现快照在 retain 后取，回滚不恢复）。
        assert!(store.delete("c2").is_err());
        assert_eq!(store.list().len(), 2);
        assert!(store.get("c2").is_some());
        assert_eq!(store.active_connection().unwrap().id.as_str(), "c2");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_load_migrates_plaintext_api_key_to_secret_ref() {
        let dir = std::env::temp_dir().join(format!(
            "storyforge_test_conn_migrate_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();

        let stored = StoredConnection {
            id: "legacy".into(),
            connection: make_conn("legacy"),
            created_at: "2026-07-06 00:00:00".into(),
            last_used_at: None,
        };
        let file = ConnectionsFile {
            active_id: Some("legacy".into()),
            connections: vec![stored],
        };
        storyforge_infra_util::atomic_write_json(&dir.join("connections.json"), &file).unwrap();

        let secret_store = Arc::new(MemorySecretStore::default());
        let store = ConnectionStore::new_with_secret_store(&dir, secret_store);
        let raw = std::fs::read_to_string(dir.join("connections.json")).unwrap();
        assert!(!raw.contains("sk-test"));
        assert!(raw.contains(storyforge_infra_util::secret_store::SECRET_REF_PREFIX));
        assert_eq!(store.active_connection().unwrap().api_key, "sk-test");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_load_recovers_tmp_and_migrates_plaintext_api_key_to_secret_ref() {
        let dir = std::env::temp_dir().join(format!(
            "storyforge_test_conn_tmp_migrate_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();

        let stored = StoredConnection {
            id: "legacy-tmp".into(),
            connection: make_conn("legacy-tmp"),
            created_at: "2026-07-06 00:00:00".into(),
            last_used_at: None,
        };
        let file = ConnectionsFile {
            active_id: Some("legacy-tmp".into()),
            connections: vec![stored],
        };
        let path = dir.join("connections.json");
        std::fs::write(&path, "{ invalid").unwrap();
        storyforge_infra_util::atomic_write_json(
            &PathBuf::from(format!("{}.tmp", path.display())),
            &file,
        )
        .unwrap();

        let secret_store = Arc::new(MemorySecretStore::default());
        let store = ConnectionStore::new_with_secret_store(&dir, secret_store);
        let raw = std::fs::read_to_string(&path).unwrap();

        assert!(!raw.contains("sk-test"));
        assert!(raw.contains(storyforge_infra_util::secret_store::SECRET_REF_PREFIX));
        assert!(!path.with_extension("json.corrupt").exists());
        assert_eq!(store.active_connection().unwrap().api_key, "sk-test");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_delete_removes_secret_ref() {
        let (store, secret_store, dir) = temp_store_with_secret_store();
        store.save(make_conn("delete-me")).unwrap();
        let secret_ref = store.get("delete-me").unwrap().connection.api_key;

        assert!(store.delete("delete-me").unwrap());
        assert!(secret_store.get_secret(&secret_ref).is_err());
        assert_eq!(
            secret_store.deleted.lock().unwrap().as_slice(),
            &[secret_ref]
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_update_existing_keeps_key_when_api_key_empty() {
        let (store, secret_store, dir) = temp_store_with_secret_store();
        store.save(make_conn("edit-me")).unwrap();
        let secret_ref = store.get("edit-me").unwrap().connection.api_key.clone();

        let mut next = make_conn("edit-me");
        next.name = "renamed".into();
        next.model = "deepseek-v3".into();
        next.api_key = String::new(); // 留空 = 不改 key
        let resolved = store.update_existing("edit-me", next).unwrap();

        assert_eq!(resolved.name, "renamed");
        assert_eq!(resolved.model, "deepseek-v3");
        assert_eq!(resolved.api_key, "sk-test");
        assert_eq!(store.get("edit-me").unwrap().connection.api_key, secret_ref);
        assert_eq!(secret_store.get_secret(&secret_ref).unwrap(), "sk-test");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_update_existing_replaces_key_when_provided() {
        let (store, secret_store, dir) = temp_store_with_secret_store();
        store.save(make_conn("edit-key")).unwrap();
        let secret_ref = store.get("edit-key").unwrap().connection.api_key.clone();

        let mut next = make_conn("edit-key");
        next.api_key = "sk-new-key".into();
        let resolved = store.update_existing("edit-key", next).unwrap();

        assert_eq!(resolved.api_key, "sk-new-key");
        assert_eq!(
            secret_store.get_secret(&secret_ref).unwrap(),
            "sk-new-key",
            "same SecretRef 槽位覆盖为新 key"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_update_existing_missing_id_errors() {
        let store = temp_store();
        let err = store
            .update_existing("nope", make_conn("nope"))
            .unwrap_err();
        assert!(err.contains("不存在"), "err={err}");
    }
}

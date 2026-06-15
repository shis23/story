/// LLM 连接存储（JSON 文件持久化）
///
/// 存储位置：data/connections.json
/// 结构：{ active_id: Option<String>, connections: Vec<StoredConnection> }
///
/// 注意：API key 当前明文存储（桌面开发阶段）。
/// Android 阶段需改为 Keystore + SecretRef（设计 §5）。
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Mutex;

use storyforge_domain::llm::LlmConnection;

/// 已存储的连接（含 api_key）
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
    inner: Mutex<ConnectionsFile>,
}

impl ConnectionStore {
    /// 初始化（从文件加载或新建）
    pub fn new(app_data_dir: &PathBuf) -> Self {
        let path = app_data_dir.join("connections.json");
        let file = if path.exists() {
            let data = std::fs::read_to_string(&path).unwrap_or_default();
            serde_json::from_str(&data).unwrap_or_default()
        } else {
            ConnectionsFile::default()
        };
        Self {
            path,
            inner: Mutex::new(file),
        }
    }

    /// 保存连接（新建或更新）
    pub fn save(&self, connection: LlmConnection) -> StoredConnection {
        let mut file = self.inner.lock().unwrap();

        // 若已存在同 id，更新；否则新增
        let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
        let conn_id = connection.id.as_str().to_string();

        if let Some(existing) = file
            .connections
            .iter_mut()
            .find(|c| c.id == conn_id)
        {
            existing.connection = connection.clone();
            existing.last_used_at = Some(now.clone());
            let updated = existing.clone();
            self.persist(&file);
            return updated;
        }

        let stored = StoredConnection {
            id: conn_id,
            connection,
            created_at: now,
            last_used_at: None,
        };
        file.connections.push(stored.clone());
        self.persist(&file);
        stored
    }

    /// 列出所有连接
    pub fn list(&self) -> Vec<StoredConnection> {
        self.inner.lock().unwrap().connections.clone()
    }

    /// 获取单个连接
    pub fn get(&self, id: &str) -> Option<StoredConnection> {
        self.inner
            .lock()
            .unwrap()
            .connections
            .iter()
            .find(|c| c.id == id)
            .cloned()
    }

    /// 删除连接（若是活跃的，同时清除 active_id）
    pub fn delete(&self, id: &str) -> bool {
        let mut file = self.inner.lock().unwrap();
        let before = file.connections.len();
        file.connections.retain(|c| c.id != id);
        if file.connections.len() < before {
            // 若删除的是活跃连接，清除 active_id
            if file.active_id.as_deref() == Some(id) {
                file.active_id = None;
            }
            self.persist(&file);
            true
        } else {
            false
        }
    }

    /// 获取当前活跃连接 ID
    #[allow(dead_code)]
    pub fn active_id(&self) -> Option<String> {
        self.inner.lock().unwrap().active_id.clone()
    }

    /// 设置活跃连接 ID（会校验该 id 存在，并更新 last_used_at）
    ///
    /// 返回对应的 LlmConnection（供调用方构造 client）。
    pub fn set_active(&self, id: &str) -> Option<LlmConnection> {
        let mut file = self.inner.lock().unwrap();
        let stored = file.connections.iter_mut().find(|c| c.id == id)?;
        stored.last_used_at =
            Some(chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string());
        let conn = stored.connection.clone();
        file.active_id = Some(id.to_string());
        self.persist(&file);
        Some(conn)
    }

    /// 获取活跃连接（若 active_id 存在且对应连接存在）
    pub fn active_connection(&self) -> Option<LlmConnection> {
        let file = self.inner.lock().unwrap();
        let active_id = file.active_id.as_ref()?;
        file.connections
            .iter()
            .find(|c| &c.id == active_id)
            .map(|c| c.connection.clone())
    }

    fn persist(&self, file: &ConnectionsFile) {
        if let Some(parent) = self.path.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                tracing::error!("创建连接数据目录失败: {e}");
                return;
            }
        }
        let json = match serde_json::to_string_pretty(file) {
            Ok(j) => j,
            Err(e) => {
                tracing::error!("序列化连接数据失败: {e}");
                return;
            }
        };
        // 原子写入：先写临时文件，再 rename（防崩溃导致文件损坏）
        let tmp_path = self.path.with_extension("json.tmp");
        if let Err(e) = std::fs::write(&tmp_path, &json) {
            tracing::error!("写入连接临时文件失败: {e}");
            return;
        }
        if let Err(e) = std::fs::rename(&tmp_path, &self.path) {
            tracing::error!("连接文件 rename 失败，尝试直接写入: {e}");
            let _ = std::fs::write(&self.path, json);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use storyforge_domain::llm::{LlmProtocol, SamplingParams, ToolMode};

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
        let dir = std::env::temp_dir().join(format!(
            "storyforge_test_conn_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        ConnectionStore::new(&dir)
    }

    #[test]
    fn test_save_list_delete() {
        let store = temp_store();
        store.save(make_conn("deepseek-1"));
        store.save(make_conn("deepseek-2"));

        assert_eq!(store.list().len(), 2);
        assert!(store.get("deepseek-1").is_some());
        assert!(store.get("nonexistent").is_none());

        assert!(store.delete("deepseek-1"));
        assert_eq!(store.list().len(), 1);
        assert!(!store.delete("nonexistent"));
    }

    #[test]
    fn test_active_id_set_and_clear() {
        let store = temp_store();
        store.save(make_conn("c1"));
        store.save(make_conn("c2"));

        assert!(store.active_id().is_none());

        // 设 c1 为活跃
        let conn = store.set_active("c1");
        assert!(conn.is_some());
        assert_eq!(store.active_id().as_deref(), Some("c1"));
        assert!(store.active_connection().is_some());

        // 删除活跃的 c1，active_id 应清除
        assert!(store.delete("c1"));
        assert!(store.active_id().is_none());
        assert!(store.active_connection().is_none());
    }

    #[test]
    fn test_persistence_across_instances() {
        let dir = std::env::temp_dir().join(format!(
            "storyforge_test_conn_persist_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();

        {
            let store = ConnectionStore::new(&dir);
            store.save(make_conn("persist-1"));
            store.set_active("persist-1");
        }

        // 新实例从同一文件加载
        let store2 = ConnectionStore::new(&dir);
        assert_eq!(store2.list().len(), 1);
        assert_eq!(store2.active_id().as_deref(), Some("persist-1"));

        let _ = std::fs::remove_dir_all(&dir);
    }
}

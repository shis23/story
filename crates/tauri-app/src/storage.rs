use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::CharacterInfo;

#[path = "json_store.rs"]
pub(crate) mod json_store;

/// 已存储的角色卡
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredCharacter {
    pub id: String,
    pub info: CharacterInfo,
    /// 导入时间
    pub imported_at: String,
}

/// 角色卡存储（JSON 文件）
pub struct CharacterStore {
    path: PathBuf,
    inner: Mutex<Vec<StoredCharacter>>,
}

impl CharacterStore {
    /// 初始化存储（从文件加载或新建）
    pub fn new(app_data_dir: &Path) -> Self {
        let path = app_data_dir.join("characters.json");
        let characters: Vec<StoredCharacter> = json_store::load_json_with_tmp_backup_or_default(
            &path,
            |e| tracing::warn!("角色卡 JSON 解析失败({e})，尝试 .tmp 备份"),
            |path, e| {
                tracing::error!(
                    "角色卡 JSON 主文件和 .tmp 备份均损坏，文件: {}, 错误: {}. 已保存 .corrupt 备份",
                    path.display(),
                    e
                )
            },
        );
        Self {
            path,
            inner: Mutex::new(characters),
        }
    }

    /// 保存角色卡（导入时调用）
    pub fn save(&self, info: CharacterInfo) -> Result<StoredCharacter, String> {
        let id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
        let stored = StoredCharacter {
            id: id.clone(),
            info,
            imported_at: now,
        };

        let mut chars = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        chars.push(stored.clone());
        self.persist(&chars)?;
        Ok(stored)
    }

    /// 列出所有角色卡（元数据）
    pub fn list(&self) -> Vec<StoredCharacter> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }

    /// 获取单个角色卡
    pub fn get(&self, id: &str) -> Option<StoredCharacter> {
        self.inner
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .iter()
            .find(|c| c.id == id)
            .cloned()
    }

    /// 删除角色卡
    pub fn delete(&self, id: &str) -> Result<bool, String> {
        let mut chars = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        let before = chars.len();
        chars.retain(|c| c.id != id);
        if chars.len() < before {
            self.persist(&chars)?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// 更新世界书条目路由
    pub fn update_world_info_route(
        &self,
        id: &str,
        entry_index: usize,
        new_route: &str,
    ) -> Result<(), String> {
        let mut chars = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        let char = chars
            .iter_mut()
            .find(|c| c.id == id)
            .ok_or_else(|| format!("角色卡不存在: {id}"))?;

        let entry = char
            .info
            .world_info_entries
            .get_mut(entry_index)
            .ok_or_else(|| format!("世界书条目索引越界: {entry_index}"))?;

        entry.route = new_route.to_string();
        self.persist(&chars)?;
        Ok(())
    }

    /// 更新世界书条目的 keys / content / constant / is_global / depth / order
    #[allow(clippy::too_many_arguments)]
    pub fn update_world_info_entry(
        &self,
        id: &str,
        entry_index: usize,
        keys: Vec<String>,
        content: String,
        constant: bool,
        is_global: bool,
        depth: i32,
        order: i32,
    ) -> Result<(), String> {
        let mut chars = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        let char = chars
            .iter_mut()
            .find(|c| c.id == id)
            .ok_or_else(|| format!("角色卡不存在: {id}"))?;

        let entry = char
            .info
            .world_info_entries
            .get_mut(entry_index)
            .ok_or_else(|| format!("世界书条目索引越界: {entry_index}"))?;

        entry.keys = keys;
        entry.content = content;
        entry.constant = constant;
        entry.is_global = is_global;
        entry.depth = depth;
        entry.order = order;
        // 同步更新计数
        char.info.world_info_count = char.info.world_info_entries.len();
        char.info.has_world_info = !char.info.world_info_entries.is_empty();
        self.persist(&chars)?;
        Ok(())
    }

    /// 新增世界书条目，返回新条目的索引
    pub fn add_world_info_entry(
        &self,
        id: &str,
        keys: Vec<String>,
        content: String,
        constant: bool,
        is_global: bool,
    ) -> Result<usize, String> {
        let mut chars = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        let char = chars
            .iter_mut()
            .find(|c| c.id == id)
            .ok_or_else(|| format!("角色卡不存在: {id}"))?;

        // 新条目默认路由：蓝灯→Constant，绿灯→Selective
        let route = if constant { "Constant" } else { "Selective" }.to_string();
        let entry = crate::WorldInfoEntryInfo {
            keys,
            content,
            constant,
            route,
            is_global,
            depth: 2,
            order: 100,
        };
        char.info.world_info_entries.push(entry);
        let new_index = char.info.world_info_entries.len() - 1;
        char.info.world_info_count = char.info.world_info_entries.len();
        char.info.has_world_info = true;
        self.persist(&chars)?;
        Ok(new_index)
    }

    /// 删除世界书条目
    pub fn delete_world_info_entry(&self, id: &str, entry_index: usize) -> Result<(), String> {
        let mut chars = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        let char = chars
            .iter_mut()
            .find(|c| c.id == id)
            .ok_or_else(|| format!("角色卡不存在: {id}"))?;

        if entry_index >= char.info.world_info_entries.len() {
            return Err(format!("世界书条目索引越界: {entry_index}"));
        }
        char.info.world_info_entries.remove(entry_index);
        char.info.world_info_count = char.info.world_info_entries.len();
        char.info.has_world_info = !char.info.world_info_entries.is_empty();
        self.persist(&chars)?;
        Ok(())
    }

    /// 批量替换世界书条目（meta_accept_patch 持久化用）
    pub fn update_world_info_entries_bulk(
        &self,
        id: &str,
        entries: Vec<crate::WorldInfoEntryInfo>,
    ) -> Result<(), String> {
        let mut chars = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        let char = chars
            .iter_mut()
            .find(|c| c.id == id)
            .ok_or_else(|| format!("角色卡不存在: {id}"))?;

        char.info.world_info_entries = entries;
        char.info.world_info_count = char.info.world_info_entries.len();
        char.info.has_world_info = !char.info.world_info_entries.is_empty();
        self.persist(&chars)?;
        Ok(())
    }

    /// 持久化到文件（原子写入：委托 infra-util）
    fn persist(&self, chars: &[StoredCharacter]) -> Result<(), String> {
        storyforge_infra_util::atomic_write_json(&self.path, chars).map_err(|e| {
            let msg = format!("持久化角色卡失败: {e}");
            tracing::error!("{msg}");
            msg
        })
    }
}

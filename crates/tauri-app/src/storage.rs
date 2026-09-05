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
    pub(crate) fn insert(&self, stored: StoredCharacter) -> Result<(), String> {
        let mut chars = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        if chars.iter().any(|c| {
            c.id == stored.id
                || c.info.source_character_id.is_some()
                    && c.info.source_character_id == stored.info.source_character_id
        }) {
            return Err("角色卡源数据已存在".into());
        }
        let mut candidate = chars.clone();
        candidate.push(stored);
        self.persist(&candidate)?;
        *chars = candidate;
        Ok(())
    }
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
        let mut candidate = chars.clone();
        candidate.push(stored.clone());
        // 候选 → 持久化 → 换入（三.9）：失败时内存保持原值。
        self.persist(&candidate)?;
        *chars = candidate;
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
        let mut candidate = chars.clone();
        candidate.retain(|c| c.id != id);
        if candidate.len() < before {
            // 候选 → 持久化 → 换入（三.9）：失败时内存保持原值。
            self.persist(&candidate)?;
            *chars = candidate;
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
        if !chars.iter().any(|c| c.id == id) {
            return Err(format!("角色卡不存在: {id}"));
        }
        let mut candidate = chars.clone();
        let char = candidate
            .iter_mut()
            .find(|c| c.id == id)
            .expect("existence pre-checked");
        let entry = char
            .info
            .world_info_entries
            .get_mut(entry_index)
            .ok_or_else(|| format!("世界书条目索引越界: {entry_index}"))?;
        entry.route = new_route.to_string();
        // 候选 → 持久化 → 换入（三.9）：失败时内存保持原值。
        self.persist(&candidate)?;
        *chars = candidate;
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
        if !chars.iter().any(|c| c.id == id) {
            return Err(format!("角色卡不存在: {id}"));
        }
        let mut candidate = chars.clone();
        let char = candidate
            .iter_mut()
            .find(|c| c.id == id)
            .expect("existence pre-checked");
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
        // 候选 → 持久化 → 换入（三.9）：失败时内存保持原值。
        self.persist(&candidate)?;
        *chars = candidate;
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
        if !chars.iter().any(|c| c.id == id) {
            return Err(format!("角色卡不存在: {id}"));
        }
        let mut candidate = chars.clone();
        let char = candidate
            .iter_mut()
            .find(|c| c.id == id)
            .expect("existence pre-checked");

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
        // 候选 → 持久化 → 换入（三.9）：失败时内存保持原值。
        self.persist(&candidate)?;
        *chars = candidate;
        Ok(new_index)
    }

    /// 删除世界书条目
    pub fn delete_world_info_entry(&self, id: &str, entry_index: usize) -> Result<(), String> {
        let mut chars = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        if !chars.iter().any(|c| c.id == id) {
            return Err(format!("角色卡不存在: {id}"));
        }
        if entry_index
            >= chars
                .iter()
                .find(|c| c.id == id)
                .map(|c| c.info.world_info_entries.len())
                .unwrap_or(0)
        {
            return Err(format!("世界书条目索引越界: {entry_index}"));
        }
        let mut candidate = chars.clone();
        let char = candidate
            .iter_mut()
            .find(|c| c.id == id)
            .expect("existence pre-checked");
        char.info.world_info_entries.remove(entry_index);
        char.info.world_info_count = char.info.world_info_entries.len();
        char.info.has_world_info = !char.info.world_info_entries.is_empty();
        // 候选 → 持久化 → 换入（三.9）：失败时内存保持原值。
        self.persist(&candidate)?;
        *chars = candidate;
        Ok(())
    }

    /// 批量替换世界书条目（meta_accept_patch 持久化用）
    pub fn update_world_info_entries_bulk(
        &self,
        id: &str,
        entries: Vec<crate::WorldInfoEntryInfo>,
    ) -> Result<(), String> {
        let mut chars = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        if !chars.iter().any(|c| c.id == id) {
            return Err(format!("角色卡不存在: {id}"));
        }
        let mut candidate = chars.clone();
        let char = candidate
            .iter_mut()
            .find(|c| c.id == id)
            .expect("existence pre-checked");
        char.info.world_info_entries = entries;
        char.info.world_info_count = char.info.world_info_entries.len();
        char.info.has_world_info = !char.info.world_info_entries.is_empty();
        // 候选 → 持久化 → 换入（三.9）：失败时内存保持原值。
        self.persist(&candidate)?;
        *chars = candidate;
        Ok(())
    }

    /// 原子批量替换多个角色的 world_info_entries（Gate 4 七审 P1）。
    ///
    /// 与 `update_world_info_entries_bulk` 不同，本方法在**一次持久化**内修改
    /// 全部目标角色——任一步失败（角色不存在）不改任何角色、不写盘，杜绝
    /// “前几个角色已更新、后一个失败”的部分提交。
    ///
    /// Gate 4 八审 P1：在**候选副本**上修改并持久化，`persist` 成功后才用候选
    /// 副本替换 `self.inner`——若 `persist` 因磁盘/权限失败返回错误，内存与文件
    /// 都保持原值，不会出现“命令报告失败、当前进程看到新值、重启后回退旧值”
    /// 的分裂状态。
    pub fn update_world_info_entries_bulk_multi(
        &self,
        entries: &[(String, Vec<crate::WorldInfoEntryInfo>)],
    ) -> Result<(), String> {
        let mut chars = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        // 先校验全部角色存在（失败则整体不改），再在候选副本上统一修改。
        for (id, _) in entries.iter() {
            if !chars.iter().any(|c| c.id == *id) {
                return Err(format!("角色卡不存在: {id}"));
            }
        }
        let mut candidate = chars.clone();
        for (id, new_entries) in entries.iter() {
            let char = candidate
                .iter_mut()
                .find(|c| c.id == *id)
                .expect("existence pre-checked");
            char.info.world_info_entries = new_entries.clone();
            char.info.world_info_count = char.info.world_info_entries.len();
            char.info.has_world_info = !char.info.world_info_entries.is_empty();
        }
        self.persist(&candidate)?;
        // 持久化成功后才替换内存态。
        *chars = candidate;
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

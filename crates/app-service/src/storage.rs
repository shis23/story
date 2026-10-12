use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::CharacterInfo;

#[path = "json_store.rs"]
pub mod json_store;

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
    pub fn insert(&self, stored: StoredCharacter) -> Result<(), String> {
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

    /// 在一张角色卡上执行变换（按存储 id 或源 `Character.id` 定位，与 SQLite
    /// `mutate_character` 同语义）。候选 → 持久化 → 换入（三.9）：变换或写盘失败时
    /// 内存与文件都保持原值。
    pub fn mutate<T>(
        &self,
        id_or_source: &str,
        f: impl FnOnce(&mut CharacterInfo) -> Result<T, String>,
    ) -> Result<T, String> {
        let mut chars = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        let index = chars
            .iter()
            .position(|c| c.id == id_or_source)
            .or_else(|| {
                chars
                    .iter()
                    .position(|c| c.info.source_character_id.as_deref() == Some(id_or_source))
            })
            .ok_or_else(|| format!("角色卡不存在: {id_or_source}"))?;
        let mut candidate = chars.clone();
        let result = f(&mut candidate[index].info)?;
        crate::services::characters::refresh_world_info_counts(&mut candidate[index].info);
        self.persist(&candidate)?;
        *chars = candidate;
        Ok(result)
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
            crate::services::characters::apply_update_world_info_entries_bulk(
                &mut char.info,
                new_entries.clone(),
            )?;
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

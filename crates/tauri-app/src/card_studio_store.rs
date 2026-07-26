use std::path::{Path, PathBuf};
use std::sync::Mutex;

use storyforge_domain::card_studio::CardProject;

use crate::storage::json_store;

pub struct CardStudioStore {
    path: PathBuf,
    inner: Mutex<Vec<CardProject>>,
}

impl CardStudioStore {
    pub fn new(app_data_dir: &Path) -> Self {
        let path = app_data_dir.join("card_projects.json");
        let projects: Vec<CardProject> = json_store::load_json_with_tmp_backup_or_default(
            &path,
            |e| tracing::warn!("card studio JSON parse failed ({e}), trying .tmp backup"),
            |path, e| {
                tracing::error!(
                    "card studio JSON and .tmp backup are corrupt, file: {}, error: {}. saved .corrupt backup",
                    path.display(),
                    e
                )
            },
        );
        Self {
            path,
            inner: Mutex::new(projects),
        }
    }

    pub fn list(&self) -> Vec<CardProject> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }

    pub fn get(&self, id: &str) -> Option<CardProject> {
        self.inner
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .iter()
            .find(|p| p.id == id)
            .cloned()
    }

    pub fn insert(&self, project: CardProject) -> Result<CardProject, String> {
        let mut projects = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        projects.push(project.clone());
        self.persist(&projects)?;
        Ok(project)
    }

    pub fn update(&self, project: CardProject) -> Result<CardProject, String> {
        let mut projects = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        let Some(slot) = projects.iter_mut().find(|p| p.id == project.id) else {
            return Err(format!("写卡项目不存在: {}", project.id));
        };
        *slot = project.clone();
        self.persist(&projects)?;
        Ok(project)
    }

    pub fn delete(&self, id: &str) -> Result<bool, String> {
        let mut projects = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        let before = projects.len();
        projects.retain(|p| p.id != id);
        if projects.len() < before {
            self.persist(&projects)?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn persist(&self, projects: &[CardProject]) -> Result<(), String> {
        storyforge_infra_util::atomic_write_json(&self.path, projects).map_err(|e| {
            let msg = format!("写卡项目落盘失败: {e}");
            tracing::error!("{msg}");
            msg
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use storyforge_domain::card_studio::CardProject;

    #[test]
    fn insert_get_update_delete_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let store = CardStudioStore::new(dir.path());
        let p = CardProject::new_from_scratch("demo", "brief");
        let id = p.id.clone();
        store.insert(p).unwrap();
        assert_eq!(store.list().len(), 1);
        let mut got = store.get(&id).unwrap();
        got.brief = "updated".into();
        store.update(got).unwrap();
        assert_eq!(store.get(&id).unwrap().brief, "updated");
        assert!(store.delete(&id).unwrap());
        assert!(store.get(&id).is_none());
    }

    #[test]
    fn novel_project_roundtrip_without_full_text_when_large() {
        let dir = tempfile::tempdir().unwrap();
        let store = CardStudioStore::new(dir.path());
        let novel = "测".repeat(90_000);
        let p = CardProject::new_from_novel("大书项目", "改编", "巨著", novel);
        assert!(p.novel_text.is_none());
        assert!(!p.novel_excerpts.is_empty());
        let id = p.id.clone();
        store.insert(p).unwrap();
        let got = store.get(&id).unwrap();
        assert!(got.novel_text.is_none());
        assert!(!got.novel_excerpts.is_empty());
    }
}

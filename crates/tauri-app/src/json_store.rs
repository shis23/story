use serde::de::DeserializeOwned;
use std::path::{Path, PathBuf};

pub(crate) fn load_json_with_tmp_backup_or_default<T>(
    path: &Path,
    on_parse_error: impl FnOnce(&serde_json::Error),
    on_recovery_failed: impl FnOnce(&Path, &serde_json::Error),
) -> T
where
    T: DeserializeOwned + Default,
{
    if !path.exists() {
        return T::default();
    }

    let data = match std::fs::read_to_string(path) {
        Ok(data) => data,
        Err(io_error) => {
            // V4：文件存在但读不出来 ≠ 首次运行——冻结写入 + 登记阻断事件，
            // 否则下一次保存会把内存里的默认空集写回、覆盖可能可抢救的数据。
            crate::storage_health::record_unrecoverable(
                path,
                &format!("读文件失败: {io_error}"),
                None,
            );
            return T::default();
        }
    };

    match serde_json::from_str(&data) {
        Ok(value) => value,
        Err(error) => {
            on_parse_error(&error);
            let tmp = PathBuf::from(format!("{}.tmp", path.display()));
            if let Ok(tmp_data) = std::fs::read_to_string(&tmp)
                && let Ok(value) = serde_json::from_str(&tmp_data)
            {
                // 数据无损恢复：仅登记提示事件，下次保存会用好数据重写主文件。
                crate::storage_health::record_tmp_recovered(path, &error.to_string());
                return value;
            }

            on_recovery_failed(path, &error);
            let corrupt = path.with_extension("json.corrupt");
            let backup_ok = std::fs::copy(path, &corrupt).is_ok();
            // V4：不可恢复损坏 → 写栅栏冻结 + 阻断事件（前端启动拦截弹恢复引导，
            // 用户确认「从空白开始」前，该文件的所有写入被拒绝）。
            crate::storage_health::record_unrecoverable(
                path,
                &error.to_string(),
                backup_ok.then_some(corrupt.as_path()),
            );
            T::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Debug, Default, Deserialize, PartialEq, Eq)]
    struct TestFile {
        values: Vec<String>,
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "storyforge_json_store_{name}_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn loads_tmp_backup_when_main_json_is_invalid() {
        let dir = temp_dir("tmp_backup");
        let path = dir.join("sample.json");
        std::fs::write(&path, "{ invalid").unwrap();
        std::fs::write(path.with_extension("json.tmp"), r#"{"values":["tmp"]}"#).unwrap();

        let loaded: TestFile = load_json_with_tmp_backup_or_default(
            &path,
            |_| {},
            |_, _| panic!("tmp should recover"),
        );

        assert_eq!(
            loaded,
            TestFile {
                values: vec!["tmp".into()]
            }
        );
        assert!(!path.with_extension("json.corrupt").exists());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn copies_corrupt_backup_when_main_and_tmp_are_invalid() {
        let dir = temp_dir("corrupt_backup");
        let path = dir.join("sample.json");
        std::fs::write(&path, "{ invalid").unwrap();
        std::fs::write(path.with_extension("json.tmp"), "{ also invalid").unwrap();

        let mut recovery_failed = false;
        let loaded: TestFile =
            load_json_with_tmp_backup_or_default(&path, |_| {}, |_, _| recovery_failed = true);

        assert_eq!(loaded, TestFile::default());
        assert!(recovery_failed);
        assert_eq!(
            std::fs::read_to_string(path.with_extension("json.corrupt")).unwrap(),
            "{ invalid"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}

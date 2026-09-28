//! 共享的 fsync 助手（S-20：cutover/rollback 曾各有一份语义不同的副本）。
//!
//! cutover 与 rollback 都走「tmp → fsync → rename → fsync(文件) → fsync(父目录)」
//! 的持久化模式。历史上 rollback 的 `fsync_parent_dir` 静默吞掉 open/sync 错误，
//! 而 cutover 的会传播——同一项目内两套语义会让「rename 是否持久化」变成偶然。
//! 这里统一为 **返回 Result 并由调用方决定**：写 marker 这类提交点传播，
//! 提交点之后不可再失败的场景显式记日志。

use std::path::Path;

use crate::error::Result;

/// fsync 文件（Windows 上 FlushFileBuffers 需要写访问，因此用写方式打开）。
pub(crate) fn fsync_file(path: &Path) -> Result<()> {
    let file = std::fs::OpenOptions::new().write(true).open(path)?;
    file.sync_all()?;
    Ok(())
}

/// fsync 父目录使 rename 本身持久化；Windows 不能对目录 fsync → 恒 Ok。
#[cfg(unix)]
pub(crate) fn fsync_parent_dir(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        let dir = std::fs::File::open(parent)?;
        dir.sync_all()?;
    }
    Ok(())
}

#[cfg(not(unix))]
pub(crate) fn fsync_parent_dir(_path: &Path) -> Result<()> {
    Ok(())
}

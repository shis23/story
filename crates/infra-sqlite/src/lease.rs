//! Cross-process authority writer lease.
//!
//! Two modes:
//! - [`LeaseMode::Exclusive`]: cutover / reverse-cutover (single writer that
//!   mutates authority).
//! - [`LeaseMode::Shared`]: regular JSON or SQLite writer process at startup.
//!
//! Same-process reentrancy is allowed: a second acquisition of an already-held
//! path by the same process returns a no-op guard. Cross-process exclusion uses
//! flock (Unix) / share-mode (Windows), mirroring `CutoverLockGuard` but with
//! shared-mode support on both platforms.
//!
//! Lease file: `<data_dir>/storyforge.authority.lock`.

use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use crate::error::{Result, SqliteError};

/// Canonical authority lease filename under the data directory.
pub const AUTHORITY_LEASE_FILENAME: &str = "storyforge.authority.lock";

/// Lease acquisition mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeaseMode {
    /// Exclusive: blocks both shared and exclusive holders in other processes.
    /// Used by cutover and reverse-cutover.
    Exclusive,
    /// Shared: coexists with other shared holders; blocked by exclusive.
    /// Used by regular JSON / SQLite writer processes at startup.
    Shared,
}

/// Process-global held-lease bookkeeping for same-process reentrancy.
#[derive(Debug, Default)]
struct HeldLeaseBook {
    /// Canonical path → (mode, refcount of real OS locks held for that path).
    entries: HashMap<PathBuf, (LeaseMode, usize)>,
}

fn held_book() -> &'static Mutex<HeldLeaseBook> {
    static BOOK: OnceLock<Mutex<HeldLeaseBook>> = OnceLock::new();
    BOOK.get_or_init(|| Mutex::new(HeldLeaseBook::default()))
}

fn canonicalize_lease_path(path: &Path) -> PathBuf {
    // Best-effort: if the file does not exist yet, canonicalize the parent and
    // rejoin the filename so reentrancy keys are stable across opens.
    if let Ok(c) = fs::canonicalize(path) {
        return c;
    }
    if let Some(parent) = path.parent()
        && let Ok(parent_c) = fs::canonicalize(parent)
        && let Some(name) = path.file_name()
    {
        return parent_c.join(name);
    }
    path.to_path_buf()
}

/// RAII guard for an authority lease. Dropping releases the OS lock (unless
/// this is a same-process reentrant no-op guard).
#[derive(Debug)]
pub struct AuthorityLeaseGuard {
    path: PathBuf,
    /// When true, this guard did not take a new OS lock (same-process reentry).
    reentrant: bool,
    #[cfg(unix)]
    file: Option<File>,
    #[cfg(not(unix))]
    _file: Option<File>,
    mode: LeaseMode,
}

impl AuthorityLeaseGuard {
    /// Acquire a lease at `path` in the given mode.
    ///
    /// Same-process reentrancy: if this process already holds a lease on the
    /// same path, returns a no-op guard immediately (compatible with both
    /// Shared and Exclusive — a process may hold Exclusive and later request
    /// Shared for the same path, or vice versa, as long as it is the same
    /// process). Cross-process conflicts fail closed.
    pub fn acquire(path: impl AsRef<Path>, mode: LeaseMode) -> Result<Self> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        // Ensure the file exists before canonicalize so the key is stable.
        // 尽力而为：若其它进程持有不兼容的锁，这里失败是预期（真正的打开在
        // acquire_os_lock 中产生带上下文的错误），绝不能把裸 io 错误吞成别的语义。
        {
            let _ = OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(false)
                .open(path);
        }
        let key = canonicalize_lease_path(path);

        {
            let mut book = held_book()
                .lock()
                .map_err(|_| SqliteError::Other("authority lease book poisoned".into()))?;
            if let Some((held_mode, count)) = book.entries.get_mut(&key) {
                // Same-process reentrancy: allow regardless of mode combination.
                // The original OS lock remains; this guard is a no-op.
                *count = count.saturating_add(1);
                let _ = held_mode; // keep original mode as the OS lock mode
                return Ok(AuthorityLeaseGuard {
                    path: key,
                    reentrant: true,
                    #[cfg(unix)]
                    file: None,
                    #[cfg(not(unix))]
                    _file: None,
                    mode,
                });
            }
        }

        let guard = acquire_os_lock(path, &key, mode)?;

        {
            let mut book = held_book()
                .lock()
                .map_err(|_| SqliteError::Other("authority lease book poisoned".into()))?;
            book.entries.insert(key.clone(), (mode, 1));
        }

        Ok(guard)
    }

    /// Convenience: acquire the shared authority lease under `data_dir`.
    pub fn acquire_shared_in(data_dir: impl AsRef<Path>) -> Result<Self> {
        let path = data_dir.as_ref().join(AUTHORITY_LEASE_FILENAME);
        Self::acquire(path, LeaseMode::Shared)
    }

    /// Convenience: acquire the exclusive authority lease under `data_dir`.
    pub fn acquire_exclusive_in(data_dir: impl AsRef<Path>) -> Result<Self> {
        let path = data_dir.as_ref().join(AUTHORITY_LEASE_FILENAME);
        Self::acquire(path, LeaseMode::Exclusive)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn mode(&self) -> LeaseMode {
        self.mode
    }

    pub fn is_reentrant(&self) -> bool {
        self.reentrant
    }
}

fn acquire_os_lock(path: &Path, key: &Path, mode: LeaseMode) -> Result<AuthorityLeaseGuard> {
    #[cfg(unix)]
    {
        use std::os::unix::io::AsRawFd;
        let file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(false)
            .open(path)?;
        let flags = match mode {
            LeaseMode::Exclusive => libc::LOCK_EX | libc::LOCK_NB,
            LeaseMode::Shared => libc::LOCK_SH | libc::LOCK_NB,
        };
        let result = unsafe { libc::flock(file.as_raw_fd(), flags) };
        if result != 0 {
            return Err(SqliteError::Other(format!(
                "authority lease unavailable ({mode:?}): another process holds storyforge.authority.lock"
            )));
        }
        Ok(AuthorityLeaseGuard {
            path: key.to_path_buf(),
            reentrant: false,
            file: Some(file),
            mode,
        })
    }
    #[cfg(not(unix))]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // Windows share 模式：
        // - SHARED 租约：只申请**读**访问 + FILE_SHARE_READ——多个 shared 持有者
        //   都只读、都允许他人读，可共存；而 EXCLUSIVE 持有者（要写访问 + 拒绝
        //   一切共享）在读权限未被授予时必然打开失败。
        // - EXCLUSIVE 租约：share_mode(0) 拒绝一切并发打开。
        // 注意：不能给 SHARED 申请写访问——写入权限会与「他人可共享读」冲突，
        // 两个 shared 写者会在 Windows 上互相拒绝（os error 32）。
        let mut options = OpenOptions::new();
        // 文件由 acquire() 的尽力 pre-open 创建；这里不再 create（Windows 上
        // create 需要写访问，而 SHARED 只申请读访问，create(true) 会直接报错）。
        options.truncate(false).share_mode(share_mode(mode));
        match mode {
            LeaseMode::Exclusive => {
                options.create(true).read(true).write(true);
            }
            LeaseMode::Shared => {
                options.create(false).read(true);
            }
        }
        let file = options.open(path).map_err(|e| {
            SqliteError::Other(format!(
                "authority lease unavailable ({mode:?}): another process holds storyforge.authority.lock ({e})"
            ))
        })?;
        Ok(AuthorityLeaseGuard {
            path: key.to_path_buf(),
            reentrant: false,
            _file: Some(file),
            mode,
        })
    }
}

#[cfg(not(unix))]
fn share_mode(mode: LeaseMode) -> u32 {
    const FILE_SHARE_READ: u32 = 0x1;
    match mode {
        LeaseMode::Exclusive => 0,
        LeaseMode::Shared => FILE_SHARE_READ,
    }
}

impl Drop for AuthorityLeaseGuard {
    fn drop(&mut self) {
        let mut book = match held_book().lock() {
            Ok(b) => b,
            Err(_) => return,
        };
        if let Some((_, count)) = book.entries.get_mut(&self.path) {
            if *count > 1 {
                *count -= 1;
                // Still held by another guard in this process; keep OS lock.
                return;
            }
            book.entries.remove(&self.path);
        }
        drop(book);

        if !self.reentrant {
            self.release_os_lock();
        }
    }
}

impl AuthorityLeaseGuard {
    /// 释放 OS 层锁。Windows 关闭句柄即释放；unix 需要显式 flock LOCK_UN。
    #[cfg(unix)]
    fn release_os_lock(&self) {
        use std::os::unix::io::AsRawFd;
        if let Some(ref file) = self.file {
            unsafe {
                libc::flock(file.as_raw_fd(), libc::LOCK_UN);
            }
        }
    }

    #[cfg(not(unix))]
    fn release_os_lock(&self) {
        // 关闭句柄（drop _file）即释放 Windows 共享锁。
    }
}

/// Acquire a process-lifetime SHARED authority lease under `data_dir`.
///
/// The guard is retained in a process-global `OnceLock` so JSON and SQLite
/// writer processes keep the lease for the whole process lifetime. Subsequent
/// calls are no-ops once the lease is held (same-process reentrancy).
pub fn hold_process_shared_lease(
    data_dir: impl AsRef<Path>,
) -> Result<&'static AuthorityLeaseGuard> {
    static PROCESS_SHARED: OnceLock<AuthorityLeaseGuard> = OnceLock::new();
    if let Some(existing) = PROCESS_SHARED.get() {
        return Ok(existing);
    }
    let guard = AuthorityLeaseGuard::acquire_shared_in(data_dir)?;
    // If two threads race, the loser drops its guard; reentrancy bookkeeping
    // keeps the OS lock alive via the winner's entry.
    let _ = PROCESS_SHARED.set(guard);
    PROCESS_SHARED
        .get()
        .ok_or_else(|| SqliteError::Other("failed to pin process shared authority lease".into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn same_process_shared_reentrancy_is_noop() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(AUTHORITY_LEASE_FILENAME);
        let first = AuthorityLeaseGuard::acquire(&path, LeaseMode::Shared).unwrap();
        assert!(!first.is_reentrant());
        let second = AuthorityLeaseGuard::acquire(&path, LeaseMode::Shared).unwrap();
        assert!(second.is_reentrant());
        drop(second);
        // First still holds.
        let third = AuthorityLeaseGuard::acquire(&path, LeaseMode::Exclusive).unwrap();
        assert!(third.is_reentrant());
        drop(third);
        drop(first);
    }

    #[test]
    fn same_process_exclusive_then_shared_reentrancy() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(AUTHORITY_LEASE_FILENAME);
        let excl = AuthorityLeaseGuard::acquire(&path, LeaseMode::Exclusive).unwrap();
        let shared = AuthorityLeaseGuard::acquire(&path, LeaseMode::Shared).unwrap();
        assert!(shared.is_reentrant());
        drop(shared);
        drop(excl);
    }

    #[test]
    fn convenience_helpers_use_canonical_filename() {
        let dir = TempDir::new().unwrap();
        let g = AuthorityLeaseGuard::acquire_shared_in(dir.path()).unwrap();
        assert!(dir.path().join(AUTHORITY_LEASE_FILENAME).exists());
        drop(g);
        let g2 = AuthorityLeaseGuard::acquire_exclusive_in(dir.path()).unwrap();
        // 前一个 guard 已 drop：同路径重新获取是真实的 OS 锁（非重入）。
        assert!(!g2.is_reentrant());
        drop(g2);
    }
}

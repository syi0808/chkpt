use std::fs::{File, OpenOptions};
use std::path::Path;

use fs4::fs_std::FileExt;

use crate::error::{ChkpttError, Result};

/// A file-based project lock that provides mutual exclusion for
/// save/restore/delete operations.
///
/// The lock is held by keeping an exclusive flock on a `project.lock` file.
/// When the `ProjectLock` is dropped, the file is closed and the lock is
/// automatically released.
pub struct ProjectLock {
    _file: File,
}

impl ProjectLock {
    /// Acquire an exclusive project lock.
    ///
    /// Creates (or opens) `lock_dir/project.lock` and attempts to take an
    /// exclusive lock on it. Returns `ChkpttError::LockHeld` if another process
    /// already holds the lock.
    pub fn acquire(lock_dir: &Path) -> Result<ProjectLock> {
        let lock_path = lock_dir.join("project.lock");
        let file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(false)
            .open(&lock_path)?;

        let acquired = file.try_lock_exclusive()?;
        if !acquired {
            return Err(ChkpttError::LockHeld);
        }

        Ok(ProjectLock { _file: file })
    }
}

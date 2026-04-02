use chkpt_core::error::ChkpttError;
use chkpt_core::ops::lock::ProjectLock;
use tempfile::TempDir;

#[test]
fn test_lock_acquire_release() {
    let dir = TempDir::new().unwrap();
    let lock_dir = dir.path().join("locks");
    std::fs::create_dir_all(&lock_dir).unwrap();
    let lock = ProjectLock::acquire(&lock_dir).unwrap();
    drop(lock); // should release
}

#[test]
fn test_double_lock_fails() {
    let dir = TempDir::new().unwrap();
    let lock_dir = dir.path().join("locks");
    std::fs::create_dir_all(&lock_dir).unwrap();
    let _lock1 = ProjectLock::acquire(&lock_dir).unwrap();
    let result = ProjectLock::acquire(&lock_dir);
    assert!(matches!(result, Err(ChkpttError::LockHeld)));
}

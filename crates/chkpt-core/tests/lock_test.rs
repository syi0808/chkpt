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
    let result = ProjectLock::try_acquire(&lock_dir);
    assert!(result.is_err() || result.unwrap().is_none());
}

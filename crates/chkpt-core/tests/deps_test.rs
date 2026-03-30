use chkpt_core::attachments::deps::{archive_deps, compute_deps_key, restore_deps};
use std::fs;
use tempfile::TempDir;

#[test]
fn test_deps_key_deterministic() {
    let dir = TempDir::new().unwrap();
    let lockfile = dir.path().join("package-lock.json");
    fs::write(&lockfile, r#"{"lockfileVersion": 3}"#).unwrap();

    let key1 = compute_deps_key(&lockfile).unwrap();
    let key2 = compute_deps_key(&lockfile).unwrap();
    assert_eq!(key1, key2);
    assert_eq!(key1.len(), 16); // 16 hex chars
}

#[test]
fn test_deps_key_different_lockfiles() {
    let dir = TempDir::new().unwrap();
    let lock1 = dir.path().join("lock1.json");
    let lock2 = dir.path().join("lock2.json");
    fs::write(&lock1, "lockfile-v1").unwrap();
    fs::write(&lock2, "lockfile-v2").unwrap();

    let key1 = compute_deps_key(&lock1).unwrap();
    let key2 = compute_deps_key(&lock2).unwrap();
    assert_ne!(key1, key2);
}

#[test]
fn test_archive_and_restore_deps() {
    let dir = TempDir::new().unwrap();
    let node_modules = dir.path().join("node_modules");
    fs::create_dir_all(node_modules.join("pkg-a")).unwrap();
    fs::write(node_modules.join("pkg-a/index.js"), "module.exports = 'a'").unwrap();
    fs::create_dir_all(node_modules.join("pkg-b")).unwrap();
    fs::write(node_modules.join("pkg-b/index.js"), "module.exports = 'b'").unwrap();

    let archive_dir = dir.path().join("archives");
    fs::create_dir_all(&archive_dir).unwrap();

    let key = archive_deps(&node_modules, &archive_dir, "test-key").unwrap();

    // Delete node_modules
    fs::remove_dir_all(&node_modules).unwrap();
    assert!(!node_modules.exists());

    // Restore
    restore_deps(&node_modules, &archive_dir, &key).unwrap();

    assert!(node_modules.join("pkg-a/index.js").exists());
    assert!(node_modules.join("pkg-b/index.js").exists());
    assert_eq!(
        fs::read_to_string(node_modules.join("pkg-a/index.js")).unwrap(),
        "module.exports = 'a'"
    );
}

#[test]
fn test_archive_reuses_existing() {
    let dir = TempDir::new().unwrap();
    let node_modules = dir.path().join("node_modules");
    fs::create_dir_all(&node_modules).unwrap();
    fs::write(node_modules.join("a.js"), "code").unwrap();

    let archive_dir = dir.path().join("archives");
    fs::create_dir_all(&archive_dir).unwrap();

    let key1 = archive_deps(&node_modules, &archive_dir, "same-key").unwrap();
    let key2 = archive_deps(&node_modules, &archive_dir, "same-key").unwrap();
    assert_eq!(key1, key2);
}

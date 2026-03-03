use chkpt_core::ops::save::{save, SaveOptions};
use tempfile::TempDir;
use std::fs;

#[test]
fn test_save_basic() {
    let workspace = TempDir::new().unwrap();
    fs::write(workspace.path().join("hello.txt"), "hello").unwrap();
    fs::write(workspace.path().join("world.txt"), "world").unwrap();

    let result = save(workspace.path(), SaveOptions::default()).unwrap();
    assert!(!result.snapshot_id.is_empty());
    assert_eq!(result.stats.total_files, 2);
    assert_eq!(result.stats.new_objects, 2);
}

#[test]
fn test_save_incremental_dedup() {
    let workspace = TempDir::new().unwrap();
    fs::write(workspace.path().join("a.txt"), "content").unwrap();

    let r1 = save(workspace.path(), SaveOptions::default()).unwrap();
    assert_eq!(r1.stats.new_objects, 1);

    // Second save with no changes: no new objects
    let r2 = save(workspace.path(), SaveOptions::default()).unwrap();
    assert_eq!(r2.stats.new_objects, 0);
}

#[test]
fn test_save_detects_changes() {
    let workspace = TempDir::new().unwrap();
    fs::write(workspace.path().join("a.txt"), "v1").unwrap();
    save(workspace.path(), SaveOptions::default()).unwrap();

    // Modify file
    fs::write(workspace.path().join("a.txt"), "v2").unwrap();
    let r2 = save(workspace.path(), SaveOptions::default()).unwrap();
    assert_eq!(r2.stats.new_objects, 1);
}

#[test]
fn test_save_with_message() {
    let workspace = TempDir::new().unwrap();
    fs::write(workspace.path().join("a.txt"), "data").unwrap();

    let opts = SaveOptions { message: Some("my checkpoint".into()), ..Default::default() };
    let result = save(workspace.path(), opts).unwrap();
    assert!(!result.snapshot_id.is_empty());
}

#[test]
fn test_save_with_subdirectories() {
    let workspace = TempDir::new().unwrap();
    fs::create_dir_all(workspace.path().join("src/utils")).unwrap();
    fs::write(workspace.path().join("src/main.rs"), "fn main(){}").unwrap();
    fs::write(workspace.path().join("src/utils/helper.rs"), "fn help(){}").unwrap();

    let result = save(workspace.path(), SaveOptions::default()).unwrap();
    assert_eq!(result.stats.total_files, 2);
}

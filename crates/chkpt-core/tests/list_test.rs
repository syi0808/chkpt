use chkpt_core::ops::list::list;
use chkpt_core::ops::save::{save, SaveOptions};
use std::fs;
use tempfile::TempDir;

#[test]
fn test_list_empty() {
    let workspace = TempDir::new().unwrap();
    let result = list(workspace.path(), None).unwrap();
    assert_eq!(result.len(), 0);
}

#[test]
fn test_list_after_saves() {
    let workspace = TempDir::new().unwrap();
    fs::write(workspace.path().join("a.txt"), "data").unwrap();
    save(workspace.path(), SaveOptions::default()).unwrap();
    fs::write(workspace.path().join("a.txt"), "data2").unwrap();
    save(workspace.path(), SaveOptions::default()).unwrap();

    let result = list(workspace.path(), None).unwrap();
    assert_eq!(result.len(), 2);
    // newest first
    assert!(result[0].created_at >= result[1].created_at);
}

#[test]
fn test_list_with_limit() {
    let workspace = TempDir::new().unwrap();
    fs::write(workspace.path().join("a.txt"), "data").unwrap();
    for _ in 0..5 {
        save(workspace.path(), SaveOptions::default()).unwrap();
    }
    let result = list(workspace.path(), Some(3)).unwrap();
    assert_eq!(result.len(), 3);
}

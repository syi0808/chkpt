use chkpt_core::config::{project_id_from_path, StoreLayout};
use chkpt_core::ops::list::list;
use chkpt_core::ops::save::{save, SaveOptions};
use std::fs;
use tempfile::TempDir;

fn root_tree_hash(bytes: &[u8; 16]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

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

#[test]
fn test_list_reads_from_catalog_without_legacy_snapshot_dir() {
    let workspace = TempDir::new().unwrap();
    fs::write(workspace.path().join("a.txt"), "data").unwrap();
    let saved = save(workspace.path(), SaveOptions::default()).unwrap();

    let layout = StoreLayout::new(&project_id_from_path(workspace.path()));
    assert!(!layout.base_dir().join("snapshots").exists());

    let result = list(workspace.path(), None).unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].id, saved.snapshot_id);
    assert_ne!(root_tree_hash(&result[0].root_tree_hash), "0".repeat(32));
}

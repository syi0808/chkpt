use chkpt_core::config::{project_id_from_path, StoreLayout};
use std::path::PathBuf;

#[test]
fn test_project_id_deterministic() {
    let path = PathBuf::from("/tmp/test-workspace");
    let id1 = project_id_from_path(&path);
    let id2 = project_id_from_path(&path);
    assert_eq!(id1, id2);
    assert_eq!(id1.len(), 16); // 16 hex chars
}

#[test]
fn test_project_id_different_paths() {
    let id1 = project_id_from_path(&PathBuf::from("/tmp/a"));
    let id2 = project_id_from_path(&PathBuf::from("/tmp/b"));
    assert_ne!(id1, id2);
}

#[test]
fn test_store_layout_paths() {
    let layout = StoreLayout::from_home_dir("/tmp/chkpt-home", "abcdef1234567890");
    let base = layout.base_dir();
    assert!(base.ends_with("abcdef1234567890"));
    assert!(layout.catalog_path().ends_with("catalog.sqlite"));
    assert!(layout.trees_dir().ends_with("trees"));
    assert!(layout.packs_dir().ends_with("packs"));
    assert!(layout.locks_dir().ends_with("locks"));
}

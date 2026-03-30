use chkpt_core::config::{project_id_from_path, Guardrails, ProjectConfig, StoreLayout};
use std::path::PathBuf;
use tempfile::TempDir;

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
    let layout = StoreLayout::new("abcdef1234567890");
    let base = layout.base_dir();
    assert!(base.ends_with("abcdef1234567890"));
    assert!(layout.snapshots_dir().ends_with("snapshots"));
    assert!(layout.objects_dir().ends_with("objects"));
    assert!(layout.trees_dir().ends_with("trees"));
    assert!(layout.packs_dir().ends_with("packs"));
    assert!(layout.locks_dir().ends_with("locks"));
}

#[test]
fn test_store_layout_object_path_has_prefix_dir() {
    let layout = StoreLayout::new("abcdef1234567890");
    let hash_hex = "a3b4c5d6e7f8901234567890abcdef1234567890abcdef1234567890abcdef12";
    let path = layout.object_path(hash_hex);
    // Should be objects/a3/b4c5d6...
    let parent = path.parent().unwrap();
    assert!(parent.ends_with("a3"));
}

#[test]
fn test_guardrails_default() {
    let g = Guardrails::default();
    assert!(g.max_total_bytes > 0);
    assert!(g.max_files > 0);
    assert!(g.max_file_size > 0);
}

#[test]
fn test_project_config_roundtrip() {
    let dir = TempDir::new().unwrap();
    let config = ProjectConfig {
        project_root: PathBuf::from("/tmp/test"),
        created_at: chrono::Utc::now(),
        guardrails: Guardrails::default(),
    };
    let path = dir.path().join("config.json");
    config.save(&path).unwrap();
    let loaded = ProjectConfig::load(&path).unwrap();
    assert_eq!(loaded.project_root, config.project_root);
}

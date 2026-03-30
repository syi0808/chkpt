use chkpt_core::config::{project_id_from_path, StoreLayout};
use chkpt_core::index::FileIndex;
use chkpt_core::ops::save::{save, SaveOptions};
use chkpt_core::store::blob::BlobStore;
use chkpt_core::store::pack::pack_loose_objects;
use std::fs;
use tempfile::TempDir;

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

    let opts = SaveOptions {
        message: Some("my checkpoint".into()),
        ..Default::default()
    };
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

#[test]
fn test_save_removes_deleted_files_from_index() {
    let workspace = TempDir::new().unwrap();
    fs::write(workspace.path().join("keep.txt"), "keep").unwrap();
    fs::write(workspace.path().join("delete.txt"), "delete").unwrap();
    save(workspace.path(), SaveOptions::default()).unwrap();

    fs::remove_file(workspace.path().join("delete.txt")).unwrap();
    save(workspace.path(), SaveOptions::default()).unwrap();

    let layout = StoreLayout::new(&project_id_from_path(workspace.path()));
    let index = FileIndex::open(layout.index_path()).unwrap();
    let entries = index.entries_by_path().unwrap();

    assert!(entries.contains_key("keep.txt"));
    assert!(!entries.contains_key("delete.txt"));
}

#[test]
fn test_save_dedups_against_packed_objects() {
    let workspace = TempDir::new().unwrap();
    fs::write(workspace.path().join("a.txt"), "same").unwrap();
    save(workspace.path(), SaveOptions::default()).unwrap();

    let layout = StoreLayout::new(&project_id_from_path(workspace.path()));
    let blob_store = BlobStore::new(layout.objects_dir());
    if !blob_store.list_loose().unwrap().is_empty() {
        pack_loose_objects(&blob_store, &layout.packs_dir()).unwrap();
    }

    fs::write(workspace.path().join("b.txt"), "same").unwrap();
    let result = save(workspace.path(), SaveOptions::default()).unwrap();

    assert_eq!(result.stats.new_objects, 0);
    assert_eq!(blob_store.list_loose().unwrap().len(), 0);
}

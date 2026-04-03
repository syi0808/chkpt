use chkpt_core::config::{project_id_from_path, StoreLayout};
use chkpt_core::index::FileIndex;
use chkpt_core::ops::save::{save, SaveOptions};
use chkpt_core::store::blob::hash_content_bytes;
use chkpt_core::store::catalog::MetadataCatalog;
use chkpt_core::store::pack::{list_packs, PackReader};
use std::fs;
use tempfile::TempDir;

#[test]
fn test_save_pre_compressed_file_stored_without_recompression() {
    let workspace = TempDir::new().unwrap();
    let content = b"fake-jpeg-content-that-is-already-compressed";
    fs::write(workspace.path().join("photo.jpg"), content).unwrap();
    fs::write(workspace.path().join("code.rs"), "fn main() {}").unwrap();

    let result = save(workspace.path(), SaveOptions::default()).unwrap();
    assert_eq!(result.stats.new_objects, 2);
    assert_eq!(result.stats.total_files, 2);

    fs::remove_file(workspace.path().join("photo.jpg")).unwrap();
    fs::remove_file(workspace.path().join("code.rs")).unwrap();

    chkpt_core::ops::restore::restore(
        workspace.path(),
        &result.snapshot_id,
        chkpt_core::ops::restore::RestoreOptions::default(),
    )
    .unwrap();

    assert_eq!(fs::read(workspace.path().join("photo.jpg")).unwrap(), content);
    assert_eq!(fs::read_to_string(workspace.path().join("code.rs")).unwrap(), "fn main() {}");
}

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
    let pack_hashes = list_packs(&layout.packs_dir()).unwrap();
    assert_eq!(pack_hashes.len(), 1);
    let reader = PackReader::open(&layout.packs_dir(), &pack_hashes[0]).unwrap();
    let hash = chkpt_core::store::blob::hash_content(b"same");
    assert_eq!(reader.read(&hash).unwrap(), b"same");

    fs::write(workspace.path().join("b.txt"), "same").unwrap();
    let result = save(workspace.path(), SaveOptions::default()).unwrap();

    assert_eq!(result.stats.new_objects, 0);

    let catalog = MetadataCatalog::open(layout.catalog_path()).unwrap();
    let location = catalog
        .blob_location(&hash_content_bytes(b"same"))
        .unwrap()
        .unwrap();
    assert_eq!(location.pack_hash.as_deref(), Some(pack_hashes[0].as_str()));
}

#[test]
fn test_save_include_deps_counts_hardlinked_files_without_new_objects_per_link() {
    let workspace = TempDir::new().unwrap();
    let pkg_a = workspace.path().join("node_modules/pkg-a");
    let pkg_b = workspace.path().join("node_modules/pkg-b");
    fs::create_dir_all(&pkg_a).unwrap();
    fs::create_dir_all(&pkg_b).unwrap();

    let original = pkg_a.join("index.js");
    let alias = pkg_b.join("index.js");
    fs::write(&original, "module.exports = 'same';").unwrap();
    fs::hard_link(&original, &alias).unwrap();

    let result = save(
        workspace.path(),
        SaveOptions {
            include_deps: true,
            ..Default::default()
        },
    )
    .unwrap();

    assert_eq!(result.stats.total_files, 2);
    assert_eq!(result.stats.new_objects, 1);
}

#[test]
fn test_save_persists_catalog() {
    let workspace = TempDir::new().unwrap();
    fs::write(workspace.path().join("a.txt"), "same").unwrap();

    let result = save(workspace.path(), SaveOptions::default()).unwrap();
    assert_eq!(result.stats.total_files, 1);

    let layout = StoreLayout::new(&project_id_from_path(workspace.path()));
    assert!(layout.catalog_path().exists());

    let catalog = MetadataCatalog::open(layout.catalog_path()).unwrap();
    let manifest = catalog.snapshot_manifest(&result.snapshot_id).unwrap();
    assert_eq!(manifest.len(), 1);
}

#[test]
fn test_save_persists_manifest_for_changed_snapshots() {
    let workspace = TempDir::new().unwrap();
    fs::write(workspace.path().join("a.txt"), "v1").unwrap();
    save(workspace.path(), SaveOptions::default()).unwrap();

    fs::write(workspace.path().join("a.txt"), "v2").unwrap();
    let result = save(workspace.path(), SaveOptions::default()).unwrap();

    let layout = StoreLayout::new(&project_id_from_path(workspace.path()));
    let catalog = MetadataCatalog::open(layout.catalog_path()).unwrap();
    let manifest = catalog.snapshot_manifest(&result.snapshot_id).unwrap();
    assert_eq!(manifest.len(), 1);
}

#[test]
fn test_save_creates_index_db() {
    let workspace = TempDir::new().unwrap();
    fs::write(workspace.path().join("a.txt"), "same").unwrap();

    save(workspace.path(), SaveOptions::default()).unwrap();

    let layout = StoreLayout::new(&project_id_from_path(workspace.path()));
    assert!(layout.index_path().exists());
}

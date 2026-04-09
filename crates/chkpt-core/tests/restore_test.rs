use chkpt_core::config::{project_id_from_path, StoreLayout};
use chkpt_core::ops::restore::{restore, RestoreOptions};
use chkpt_core::ops::save::{save, SaveOptions};
use chkpt_core::store::pack::list_packs;
use std::fs;
use tempfile::TempDir;

#[test]
fn test_restore_basic() {
    // save -> modify -> restore -> verify original state
    let workspace = TempDir::new().unwrap();
    fs::write(workspace.path().join("a.txt"), "original").unwrap();
    let r = save(workspace.path(), SaveOptions::default()).unwrap();

    fs::write(workspace.path().join("a.txt"), "modified").unwrap();
    restore(workspace.path(), &r.snapshot_id, RestoreOptions::default()).unwrap();

    let content = fs::read_to_string(workspace.path().join("a.txt")).unwrap();
    assert_eq!(content, "original");
}

#[test]
fn test_restore_dry_run() {
    // dry-run returns summary without modifying workspace
    let workspace = TempDir::new().unwrap();
    fs::write(workspace.path().join("a.txt"), "v1").unwrap();
    let r = save(workspace.path(), SaveOptions::default()).unwrap();

    fs::write(workspace.path().join("a.txt"), "v2").unwrap();
    let opts = RestoreOptions {
        dry_run: true,
        ..Default::default()
    };
    let result = restore(workspace.path(), &r.snapshot_id, opts).unwrap();

    // Workspace should NOT be modified
    let content = fs::read_to_string(workspace.path().join("a.txt")).unwrap();
    assert_eq!(content, "v2");
    // Result should report changes
    assert!(result.files_changed > 0 || result.files_added > 0 || result.files_removed > 0);
}

#[test]
fn test_restore_latest() {
    // "latest" alias works
    let workspace = TempDir::new().unwrap();
    fs::write(workspace.path().join("a.txt"), "v1").unwrap();
    save(workspace.path(), SaveOptions::default()).unwrap();
    fs::write(workspace.path().join("a.txt"), "v2").unwrap();
    let _r2 = save(workspace.path(), SaveOptions::default()).unwrap();

    fs::write(workspace.path().join("a.txt"), "v3").unwrap();
    restore(workspace.path(), "latest", RestoreOptions::default()).unwrap();

    let content = fs::read_to_string(workspace.path().join("a.txt")).unwrap();
    assert_eq!(content, "v2");
}

#[test]
fn test_restore_with_added_deleted_files() {
    let workspace = TempDir::new().unwrap();
    fs::write(workspace.path().join("keep.txt"), "keep").unwrap();
    fs::write(workspace.path().join("remove.txt"), "remove").unwrap();
    let r = save(workspace.path(), SaveOptions::default()).unwrap();

    // Delete a file and add a new one
    fs::remove_file(workspace.path().join("remove.txt")).unwrap();
    fs::write(workspace.path().join("new.txt"), "new").unwrap();

    restore(workspace.path(), &r.snapshot_id, RestoreOptions::default()).unwrap();

    // Original files should be restored
    assert!(workspace.path().join("keep.txt").exists());
    assert!(workspace.path().join("remove.txt").exists());
    // New file should be removed (not in snapshot)
    assert!(!workspace.path().join("new.txt").exists());
}

#[test]
fn test_restore_with_subdirectories() {
    let workspace = TempDir::new().unwrap();
    fs::create_dir_all(workspace.path().join("src")).unwrap();
    fs::write(workspace.path().join("src/main.rs"), "fn main(){}").unwrap();
    let r = save(workspace.path(), SaveOptions::default()).unwrap();

    fs::write(workspace.path().join("src/main.rs"), "modified").unwrap();
    restore(workspace.path(), &r.snapshot_id, RestoreOptions::default()).unwrap();

    let content = fs::read_to_string(workspace.path().join("src/main.rs")).unwrap();
    assert_eq!(content, "fn main(){}");
}

#[test]
fn test_save_after_restore_stays_incremental() {
    let workspace = TempDir::new().unwrap();
    fs::write(workspace.path().join("a.txt"), "v1").unwrap();
    let snapshot = save(workspace.path(), SaveOptions::default()).unwrap();

    fs::write(workspace.path().join("a.txt"), "v2 with longer content").unwrap();
    restore(
        workspace.path(),
        &snapshot.snapshot_id,
        RestoreOptions::default(),
    )
    .unwrap();

    let resaved = save(workspace.path(), SaveOptions::default()).unwrap();
    assert_eq!(resaved.stats.new_objects, 0);
}

#[test]
fn test_restore_after_add_remove_change_keeps_follow_up_save_incremental() {
    let workspace = TempDir::new().unwrap();
    fs::write(workspace.path().join("keep.txt"), "keep-v1").unwrap();
    fs::write(workspace.path().join("remove.txt"), "remove-v1").unwrap();
    let snapshot = save(workspace.path(), SaveOptions::default()).unwrap();

    fs::write(workspace.path().join("keep.txt"), "keep-v2").unwrap();
    fs::remove_file(workspace.path().join("remove.txt")).unwrap();
    fs::write(workspace.path().join("new.txt"), "new-v1").unwrap();

    restore(
        workspace.path(),
        &snapshot.snapshot_id,
        RestoreOptions::default(),
    )
    .unwrap();

    assert_eq!(
        fs::read_to_string(workspace.path().join("keep.txt")).unwrap(),
        "keep-v1"
    );
    assert_eq!(
        fs::read_to_string(workspace.path().join("remove.txt")).unwrap(),
        "remove-v1"
    );
    assert!(!workspace.path().join("new.txt").exists());

    let resaved = save(workspace.path(), SaveOptions::default()).unwrap();
    assert_eq!(resaved.stats.new_objects, 0);
}

#[test]
fn test_restore_reads_from_packed_objects() {
    let workspace = TempDir::new().unwrap();
    fs::write(workspace.path().join("a.txt"), "v1").unwrap();
    fs::write(workspace.path().join("nested.txt"), "nested-v1").unwrap();
    let snapshot = save(workspace.path(), SaveOptions::default()).unwrap();

    let layout = StoreLayout::new(&project_id_from_path(workspace.path()));
    assert!(!list_packs(&layout.packs_dir()).unwrap().is_empty());

    fs::write(workspace.path().join("a.txt"), "v2").unwrap();
    fs::write(workspace.path().join("nested.txt"), "nested-v2").unwrap();

    restore(
        workspace.path(),
        &snapshot.snapshot_id,
        RestoreOptions::default(),
    )
    .unwrap();

    assert_eq!(
        fs::read_to_string(workspace.path().join("a.txt")).unwrap(),
        "v1"
    );
    assert_eq!(
        fs::read_to_string(workspace.path().join("nested.txt")).unwrap(),
        "nested-v1"
    );
}

#[cfg(unix)]
#[test]
fn test_restore_round_trips_symlinks() {
    use std::os::unix::fs::symlink;

    let workspace = TempDir::new().unwrap();
    fs::create_dir_all(workspace.path().join("src")).unwrap();
    fs::write(workspace.path().join("src/main.js"), "console.log('v1')").unwrap();
    symlink("src/main.js", workspace.path().join("app.js")).unwrap();

    let snapshot = save(workspace.path(), SaveOptions::default()).unwrap();

    fs::remove_file(workspace.path().join("app.js")).unwrap();
    fs::write(workspace.path().join("app.js"), "not-a-link").unwrap();

    restore(
        workspace.path(),
        &snapshot.snapshot_id,
        RestoreOptions::default(),
    )
    .unwrap();

    let metadata = fs::symlink_metadata(workspace.path().join("app.js")).unwrap();
    assert!(metadata.file_type().is_symlink());
    assert_eq!(
        fs::read_link(workspace.path().join("app.js")).unwrap(),
        std::path::PathBuf::from("src/main.js")
    );
}

#[test]
fn test_restore_deeply_nested_many_files_same_dir() {
    let workspace = TempDir::new().unwrap();
    let deep_dir = workspace.path().join("a/b/c/d/e");
    fs::create_dir_all(&deep_dir).unwrap();
    for i in 0..50 {
        fs::write(
            deep_dir.join(format!("file_{}.txt", i)),
            format!("content_{}", i),
        )
        .unwrap();
    }

    let r = save(workspace.path(), SaveOptions::default()).unwrap();

    // Remove all files
    fs::remove_dir_all(workspace.path().join("a")).unwrap();

    restore(workspace.path(), &r.snapshot_id, RestoreOptions::default()).unwrap();

    for i in 0..50 {
        let content = fs::read_to_string(deep_dir.join(format!("file_{}.txt", i))).unwrap();
        assert_eq!(content, format!("content_{}", i));
    }
}

#[test]
fn test_restore_works_without_tree_files() {
    let workspace = TempDir::new().unwrap();
    fs::write(workspace.path().join("a.txt"), "v1").unwrap();
    let snapshot = save(workspace.path(), SaveOptions::default()).unwrap();

    let layout = StoreLayout::new(&project_id_from_path(workspace.path()));
    assert!(!layout.base_dir().join("snapshots").exists());
    fs::remove_dir_all(layout.trees_dir()).unwrap();

    fs::write(workspace.path().join("a.txt"), "v2").unwrap();
    restore(
        workspace.path(),
        &snapshot.snapshot_id,
        RestoreOptions::default(),
    )
    .unwrap();

    assert_eq!(
        fs::read_to_string(workspace.path().join("a.txt")).unwrap(),
        "v1"
    );
}

#[test]
fn test_restore_removes_many_files_correctly() {
    let workspace = TempDir::new().unwrap();
    for i in 0..5 {
        fs::write(
            workspace.path().join(format!("keep_{}.txt", i)),
            format!("keep_{}", i),
        )
        .unwrap();
    }
    let snapshot = save(workspace.path(), SaveOptions::default()).unwrap();

    fs::create_dir_all(workspace.path().join("extra")).unwrap();
    for i in 0..200 {
        fs::write(
            workspace.path().join(format!("extra/file_{}.txt", i)),
            format!("extra_{}", i),
        )
        .unwrap();
    }

    let result = restore(
        workspace.path(),
        &snapshot.snapshot_id,
        RestoreOptions::default(),
    )
    .unwrap();

    assert_eq!(result.files_removed, 200);
    assert_eq!(result.files_unchanged, 5);
    assert!(!workspace.path().join("extra").exists());
    for i in 0..5 {
        assert_eq!(
            fs::read_to_string(workspace.path().join(format!("keep_{}.txt", i))).unwrap(),
            format!("keep_{}", i)
        );
    }
}

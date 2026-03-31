use chkpt_core::ops::restore::{restore, RestoreOptions};
use chkpt_core::ops::save::{save, SaveOptions};
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
    let opts = RestoreOptions { dry_run: true };
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

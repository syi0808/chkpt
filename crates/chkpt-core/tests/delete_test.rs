use chkpt_core::ops::delete::delete;
use chkpt_core::ops::list::list;
use chkpt_core::ops::restore::{restore, RestoreOptions};
use chkpt_core::ops::save::{save, SaveOptions};
use std::fs;
use tempfile::TempDir;

#[test]
fn test_delete_single_snapshot() {
    // save -> delete -> list is empty
    let workspace = TempDir::new().unwrap();
    fs::write(workspace.path().join("a.txt"), "data").unwrap();
    let r = save(workspace.path(), SaveOptions::default()).unwrap();

    delete(workspace.path(), &r.snapshot_id).unwrap();

    let snapshots = list(workspace.path(), None).unwrap();
    assert_eq!(snapshots.len(), 0);
}

#[test]
fn test_delete_preserves_other_snapshots() {
    // save A -> save B -> delete A -> B still restorable
    let workspace = TempDir::new().unwrap();
    fs::write(workspace.path().join("a.txt"), "v1").unwrap();
    let r1 = save(workspace.path(), SaveOptions::default()).unwrap();

    fs::write(workspace.path().join("a.txt"), "v2").unwrap();
    let r2 = save(workspace.path(), SaveOptions::default()).unwrap();

    delete(workspace.path(), &r1.snapshot_id).unwrap();

    let snapshots = list(workspace.path(), None).unwrap();
    assert_eq!(snapshots.len(), 1);
    assert_eq!(snapshots[0].id, r2.snapshot_id);

    // B should still be restorable
    fs::write(workspace.path().join("a.txt"), "v3").unwrap();
    restore(workspace.path(), &r2.snapshot_id, RestoreOptions::default()).unwrap();
    let content = fs::read_to_string(workspace.path().join("a.txt")).unwrap();
    assert_eq!(content, "v2");
}

#[test]
fn test_delete_shared_objects_preserved() {
    // save A with file -> save B with same file -> delete A -> B still works
    // (shared blob objects between snapshots are not deleted prematurely)
    let workspace = TempDir::new().unwrap();
    fs::write(workspace.path().join("shared.txt"), "shared content").unwrap();
    let r1 = save(workspace.path(), SaveOptions::default()).unwrap();

    fs::write(workspace.path().join("extra.txt"), "extra").unwrap();
    let r2 = save(workspace.path(), SaveOptions::default()).unwrap();

    delete(workspace.path(), &r1.snapshot_id).unwrap();

    // B should still be fully restorable (shared.txt blob still exists)
    fs::remove_file(workspace.path().join("shared.txt")).unwrap();
    fs::remove_file(workspace.path().join("extra.txt")).unwrap();
    restore(workspace.path(), &r2.snapshot_id, RestoreOptions::default()).unwrap();
    assert_eq!(
        fs::read_to_string(workspace.path().join("shared.txt")).unwrap(),
        "shared content"
    );
    assert_eq!(
        fs::read_to_string(workspace.path().join("extra.txt")).unwrap(),
        "extra"
    );
}

#[test]
fn test_delete_nonexistent() {
    let workspace = TempDir::new().unwrap();
    let result = delete(workspace.path(), "nonexistent-id");
    assert!(result.is_err());
}

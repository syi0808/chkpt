use chkpt_core::error::ChkpttError;
use chkpt_core::ops::delete::delete;
use chkpt_core::ops::list::list;
use chkpt_core::ops::restore::{restore, RestoreOptions};
use chkpt_core::ops::save::{save, SaveOptions};
use std::fs;
use tempfile::TempDir;

fn unique_prefix(snapshot_id: &str, other_snapshot_id: &str) -> String {
    let shared = snapshot_id
        .chars()
        .zip(other_snapshot_id.chars())
        .take_while(|(left, right)| left == right)
        .count();
    snapshot_id.chars().take(shared + 1).collect()
}

/// Full lifecycle: save -> list -> restore -> verify
#[test]
fn test_e2e_save_list_restore() {
    let workspace = TempDir::new().unwrap();
    fs::write(workspace.path().join("readme.md"), "# Hello").unwrap();
    fs::create_dir_all(workspace.path().join("src")).unwrap();
    fs::write(workspace.path().join("src/main.rs"), "fn main() {}").unwrap();

    // Save
    let r = save(
        workspace.path(),
        SaveOptions {
            message: Some("initial".into()),
            ..Default::default()
        },
    )
    .unwrap();

    // List (returns newest first)
    let snapshots = list(workspace.path(), None).unwrap();
    assert_eq!(snapshots.len(), 1);
    assert_eq!(snapshots[0].id, r.snapshot_id);
    assert_eq!(snapshots[0].message.as_deref(), Some("initial"));

    // Modify workspace
    fs::write(workspace.path().join("readme.md"), "# Modified").unwrap();
    fs::write(workspace.path().join("new.txt"), "new file").unwrap();

    // Restore
    let rr = restore(workspace.path(), &r.snapshot_id, RestoreOptions::default()).unwrap();
    assert!(rr.files_changed > 0 || rr.files_removed > 0);

    // Verify restored state matches original
    assert_eq!(
        fs::read_to_string(workspace.path().join("readme.md")).unwrap(),
        "# Hello"
    );
    assert_eq!(
        fs::read_to_string(workspace.path().join("src/main.rs")).unwrap(),
        "fn main() {}"
    );
    assert!(!workspace.path().join("new.txt").exists());
}

/// Multiple saves and selective restore to earlier snapshots
#[test]
fn test_e2e_multiple_saves_selective_restore() {
    let workspace = TempDir::new().unwrap();

    fs::write(workspace.path().join("a.txt"), "version 1").unwrap();
    let r1 = save(
        workspace.path(),
        SaveOptions {
            message: Some("v1".into()),
            ..Default::default()
        },
    )
    .unwrap();

    fs::write(workspace.path().join("a.txt"), "version 2").unwrap();
    fs::write(workspace.path().join("b.txt"), "added in v2").unwrap();
    let r2 = save(
        workspace.path(),
        SaveOptions {
            message: Some("v2".into()),
            ..Default::default()
        },
    )
    .unwrap();

    fs::write(workspace.path().join("a.txt"), "version 3").unwrap();

    // Restore to v1
    restore(workspace.path(), &r1.snapshot_id, RestoreOptions::default()).unwrap();
    assert_eq!(
        fs::read_to_string(workspace.path().join("a.txt")).unwrap(),
        "version 1"
    );
    assert!(!workspace.path().join("b.txt").exists());

    // Restore to v2
    restore(workspace.path(), &r2.snapshot_id, RestoreOptions::default()).unwrap();
    assert_eq!(
        fs::read_to_string(workspace.path().join("a.txt")).unwrap(),
        "version 2"
    );
    assert_eq!(
        fs::read_to_string(workspace.path().join("b.txt")).unwrap(),
        "added in v2"
    );
}

/// Delete a snapshot; remaining snapshots still work
#[test]
fn test_e2e_delete_gc_preserves_valid() {
    let workspace = TempDir::new().unwrap();

    fs::write(workspace.path().join("shared.txt"), "shared content").unwrap();
    let r1 = save(workspace.path(), SaveOptions::default()).unwrap();

    fs::write(workspace.path().join("unique.txt"), "only in v2").unwrap();
    let r2 = save(workspace.path(), SaveOptions::default()).unwrap();

    // Delete first snapshot
    delete(workspace.path(), &r1.snapshot_id).unwrap();

    // Second snapshot should still be listed and restorable
    let snaps = list(workspace.path(), None).unwrap();
    assert_eq!(snaps.len(), 1);
    assert_eq!(snaps[0].id, r2.snapshot_id);

    // Remove files from workspace, then restore from the surviving snapshot
    fs::remove_file(workspace.path().join("shared.txt")).unwrap();
    fs::remove_file(workspace.path().join("unique.txt")).unwrap();
    restore(workspace.path(), &r2.snapshot_id, RestoreOptions::default()).unwrap();
    assert_eq!(
        fs::read_to_string(workspace.path().join("shared.txt")).unwrap(),
        "shared content"
    );
    assert_eq!(
        fs::read_to_string(workspace.path().join("unique.txt")).unwrap(),
        "only in v2"
    );
}

/// Dry-run reports changes without modifying workspace
#[test]
fn test_e2e_dry_run_no_modification() {
    let workspace = TempDir::new().unwrap();
    fs::write(workspace.path().join("a.txt"), "original").unwrap();
    let r = save(workspace.path(), SaveOptions::default()).unwrap();

    fs::write(workspace.path().join("a.txt"), "changed").unwrap();
    fs::write(workspace.path().join("extra.txt"), "extra").unwrap();

    let result = restore(
        workspace.path(),
        &r.snapshot_id,
        RestoreOptions {
            dry_run: true,
            ..Default::default()
        },
    )
    .unwrap();

    // Should report changes
    assert!(result.files_changed + result.files_added + result.files_removed > 0);

    // But workspace should be unchanged
    assert_eq!(
        fs::read_to_string(workspace.path().join("a.txt")).unwrap(),
        "changed"
    );
    assert!(workspace.path().join("extra.txt").exists());
}

/// Large file count scenario with deduplication
#[test]
fn test_e2e_many_files() {
    let workspace = TempDir::new().unwrap();

    // Create 200 files across 10 directories
    for i in 0..200 {
        let dir = workspace.path().join(format!("dir_{}", i / 20));
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join(format!("file_{}.txt", i)),
            format!("content-{}", i),
        )
        .unwrap();
    }

    let r = save(workspace.path(), SaveOptions::default()).unwrap();
    assert_eq!(r.stats.total_files, 200);
    assert_eq!(r.stats.new_objects, 200);

    // Second save with no changes: all deduped
    let r2 = save(workspace.path(), SaveOptions::default()).unwrap();
    assert_eq!(r2.stats.new_objects, 0);

    // Modify some files
    for i in 0..10 {
        let dir = workspace.path().join(format!("dir_{}", i / 20));
        fs::write(
            dir.join(format!("file_{}.txt", i)),
            format!("modified-{}", i),
        )
        .unwrap();
    }

    let r3 = save(workspace.path(), SaveOptions::default()).unwrap();
    assert_eq!(r3.stats.new_objects, 10);

    // Restore to original
    restore(workspace.path(), &r.snapshot_id, RestoreOptions::default()).unwrap();

    // Verify a few files
    let content = fs::read_to_string(workspace.path().join("dir_0/file_0.txt")).unwrap();
    assert_eq!(content, "content-0");
    let content5 = fs::read_to_string(workspace.path().join("dir_0/file_5.txt")).unwrap();
    assert_eq!(content5, "content-5");
    let content199 = fs::read_to_string(workspace.path().join("dir_9/file_199.txt")).unwrap();
    assert_eq!(content199, "content-199");
}

/// Save respects .chkptignore patterns
#[test]
fn test_e2e_chkptignore() {
    let workspace = TempDir::new().unwrap();
    fs::write(workspace.path().join("keep.txt"), "keep").unwrap();
    fs::write(workspace.path().join("ignore.log"), "ignore").unwrap();
    fs::create_dir_all(workspace.path().join("build")).unwrap();
    fs::write(workspace.path().join("build/output.o"), "binary").unwrap();
    fs::write(workspace.path().join(".chkptignore"), "*.log\nbuild/\n").unwrap();

    let r = save(workspace.path(), SaveOptions::default()).unwrap();
    // Should only save keep.txt and .chkptignore (ignore.log and build/ excluded)
    assert_eq!(r.stats.total_files, 2);
}

/// Empty workspace produces a valid snapshot
#[test]
fn test_e2e_empty_workspace() {
    let workspace = TempDir::new().unwrap();
    let r = save(workspace.path(), SaveOptions::default()).unwrap();
    assert_eq!(r.stats.total_files, 0);
    assert_eq!(r.stats.new_objects, 0);

    // Should be listable
    let snaps = list(workspace.path(), None).unwrap();
    assert_eq!(snaps.len(), 1);

    // Should be restorable (no-op)
    let rr = restore(workspace.path(), &r.snapshot_id, RestoreOptions::default()).unwrap();
    assert_eq!(rr.files_added, 0);
    assert_eq!(rr.files_changed, 0);
    assert_eq!(rr.files_removed, 0);
}

/// Restore "latest" alias picks the most recent snapshot
#[test]
fn test_e2e_restore_latest() {
    let workspace = TempDir::new().unwrap();
    fs::write(workspace.path().join("a.txt"), "v1").unwrap();
    save(workspace.path(), SaveOptions::default()).unwrap();

    fs::write(workspace.path().join("a.txt"), "v2").unwrap();
    save(workspace.path(), SaveOptions::default()).unwrap();

    fs::write(workspace.path().join("a.txt"), "v3").unwrap();

    restore(workspace.path(), "latest", RestoreOptions::default()).unwrap();
    assert_eq!(
        fs::read_to_string(workspace.path().join("a.txt")).unwrap(),
        "v2"
    );
}

#[test]
fn test_e2e_restore_unique_prefix() {
    let workspace = TempDir::new().unwrap();
    fs::write(workspace.path().join("a.txt"), "v1").unwrap();
    let r1 = save(workspace.path(), SaveOptions::default()).unwrap();

    fs::write(workspace.path().join("a.txt"), "v2").unwrap();
    let r2 = save(workspace.path(), SaveOptions::default()).unwrap();

    fs::write(workspace.path().join("a.txt"), "v3").unwrap();

    let prefix = unique_prefix(&r1.snapshot_id, &r2.snapshot_id);
    restore(workspace.path(), &prefix, RestoreOptions::default()).unwrap();
    assert_eq!(
        fs::read_to_string(workspace.path().join("a.txt")).unwrap(),
        "v1"
    );

    fs::write(workspace.path().join("a.txt"), "v4").unwrap();
    let latest_prefix = unique_prefix(&r2.snapshot_id, &r1.snapshot_id);
    restore(workspace.path(), &latest_prefix, RestoreOptions::default()).unwrap();
    assert_eq!(
        fs::read_to_string(workspace.path().join("a.txt")).unwrap(),
        "v2"
    );
}

#[test]
fn test_e2e_restore_ambiguous_prefix_errors() {
    let workspace = TempDir::new().unwrap();
    fs::write(workspace.path().join("a.txt"), "v1").unwrap();
    let r1 = save(workspace.path(), SaveOptions::default()).unwrap();

    fs::write(workspace.path().join("a.txt"), "v2").unwrap();
    let r2 = save(workspace.path(), SaveOptions::default()).unwrap();

    let shared_prefix: String = r1
        .snapshot_id
        .chars()
        .zip(r2.snapshot_id.chars())
        .take_while(|(left, right)| left == right)
        .map(|(ch, _)| ch)
        .collect();
    assert!(!shared_prefix.is_empty());

    let err = restore(
        workspace.path(),
        &shared_prefix,
        RestoreOptions {
            dry_run: true,
            ..Default::default()
        },
    )
    .unwrap_err();
    assert!(
        matches!(err, ChkpttError::Other(message) if message.contains("Ambiguous snapshot prefix"))
    );
}

#[test]
fn test_e2e_repeated_restore_save_delete_lifecycle() {
    let workspace = TempDir::new().unwrap();

    fs::create_dir_all(workspace.path().join("src")).unwrap();
    fs::write(
        workspace.path().join("src/main.rs"),
        "fn main() { println!(\"v1\"); }",
    )
    .unwrap();
    let first = save(
        workspace.path(),
        SaveOptions {
            message: Some("first".into()),
            ..Default::default()
        },
    )
    .unwrap();

    fs::write(
        workspace.path().join("src/main.rs"),
        "fn main() { println!(\"v2\"); }",
    )
    .unwrap();
    fs::write(workspace.path().join("README.md"), "# v2").unwrap();
    let second = save(
        workspace.path(),
        SaveOptions {
            message: Some("second".into()),
            ..Default::default()
        },
    )
    .unwrap();

    restore(
        workspace.path(),
        &first.snapshot_id,
        RestoreOptions::default(),
    )
    .unwrap();
    assert_eq!(
        fs::read_to_string(workspace.path().join("src/main.rs")).unwrap(),
        "fn main() { println!(\"v1\"); }"
    );
    assert!(!workspace.path().join("README.md").exists());

    let resaved = save(
        workspace.path(),
        SaveOptions {
            message: Some("resaved".into()),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(resaved.stats.new_objects, 0);

    delete(workspace.path(), &first.snapshot_id).unwrap();
    delete(workspace.path(), &resaved.snapshot_id).unwrap();

    fs::write(
        workspace.path().join("src/main.rs"),
        "fn main() { println!(\"mutated\"); }",
    )
    .unwrap();
    restore(
        workspace.path(),
        &second.snapshot_id,
        RestoreOptions::default(),
    )
    .unwrap();
    assert_eq!(
        fs::read_to_string(workspace.path().join("src/main.rs")).unwrap(),
        "fn main() { println!(\"v2\"); }"
    );
    assert_eq!(
        fs::read_to_string(workspace.path().join("README.md")).unwrap(),
        "# v2"
    );
}

/// Deleting all snapshots leaves an empty list
#[test]
fn test_e2e_delete_all() {
    let workspace = TempDir::new().unwrap();
    fs::write(workspace.path().join("a.txt"), "data").unwrap();

    let r1 = save(workspace.path(), SaveOptions::default()).unwrap();
    let r2 = save(workspace.path(), SaveOptions::default()).unwrap();

    delete(workspace.path(), &r1.snapshot_id).unwrap();
    delete(workspace.path(), &r2.snapshot_id).unwrap();

    let snaps = list(workspace.path(), None).unwrap();
    assert_eq!(snaps.len(), 0);
}

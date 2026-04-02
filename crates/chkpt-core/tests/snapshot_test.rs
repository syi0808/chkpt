use chkpt_core::store::snapshot::{Snapshot, SnapshotStats, SnapshotStore};
use tempfile::TempDir;

#[test]
fn test_snapshot_save_and_load() {
    let dir = TempDir::new().unwrap();
    let store = SnapshotStore::new(dir.path().to_path_buf());
    let snap = Snapshot::new(
        Some("test save".into()),
        [0u8; 32],
        None,
        SnapshotStats {
            total_files: 10,
            total_bytes: 1000,
            new_objects: 5,
        },
    );
    let id = snap.id.clone();
    store.save(&snap).unwrap();
    let loaded = store.load(&id).unwrap();
    assert_eq!(loaded.id, id);
    assert_eq!(loaded.message.as_deref(), Some("test save"));
    assert_eq!(loaded.root_tree_hash, [0u8; 32]);
}

#[test]
fn test_snapshot_list_sorted() {
    let dir = TempDir::new().unwrap();
    let store = SnapshotStore::new(dir.path().to_path_buf());
    for i in 0..3 {
        let snap = Snapshot::new(
            Some(format!("snap {}", i)),
            [i as u8; 32],
            None,
            SnapshotStats {
                total_files: 0,
                total_bytes: 0,
                new_objects: 0,
            },
        );
        store.save(&snap).unwrap();
    }
    let list = store.list(None).unwrap();
    assert_eq!(list.len(), 3);
    // Should be newest first
    assert!(list[0].created_at >= list[1].created_at);
}

#[test]
fn test_snapshot_delete() {
    let dir = TempDir::new().unwrap();
    let store = SnapshotStore::new(dir.path().to_path_buf());
    let snap = Snapshot::new(
        None,
        [0u8; 32],
        None,
        SnapshotStats {
            total_files: 0,
            total_bytes: 0,
            new_objects: 0,
        },
    );
    let id = snap.id.clone();
    store.save(&snap).unwrap();
    store.delete(&id).unwrap();
    assert!(store.load(&id).is_err());
}

#[test]
fn test_snapshot_list_with_limit() {
    let dir = TempDir::new().unwrap();
    let store = SnapshotStore::new(dir.path().to_path_buf());
    for _ in 0..5 {
        let snap = Snapshot::new(
            None,
            [0u8; 32],
            None,
            SnapshotStats {
                total_files: 0,
                total_bytes: 0,
                new_objects: 0,
            },
        );
        store.save(&snap).unwrap();
    }
    let list = store.list(Some(3)).unwrap();
    assert_eq!(list.len(), 3);
}

#[test]
fn test_snapshot_latest() {
    let dir = TempDir::new().unwrap();
    let store = SnapshotStore::new(dir.path().to_path_buf());
    let snap1 = Snapshot::new(
        Some("first".into()),
        [1u8; 32],
        None,
        SnapshotStats {
            total_files: 0,
            total_bytes: 0,
            new_objects: 0,
        },
    );
    store.save(&snap1).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(10));
    let snap2 = Snapshot::new(
        Some("second".into()),
        [2u8; 32],
        None,
        SnapshotStats {
            total_files: 0,
            total_bytes: 0,
            new_objects: 0,
        },
    );
    let id2 = snap2.id.clone();
    store.save(&snap2).unwrap();
    let latest = store.latest().unwrap().unwrap();
    assert_eq!(latest.id, id2);
}

#[test]
fn test_snapshot_latest_falls_back_when_pointer_is_stale() {
    let dir = TempDir::new().unwrap();
    let store = SnapshotStore::new(dir.path().to_path_buf());
    let snap = Snapshot::new(
        Some("only".into()),
        [7u8; 32],
        None,
        SnapshotStats {
            total_files: 0,
            total_bytes: 0,
            new_objects: 0,
        },
    );
    let id = snap.id.clone();
    store.save(&snap).unwrap();
    std::fs::write(dir.path().join(".latest"), "missing-snapshot").unwrap();

    let latest = store.latest().unwrap().unwrap();
    assert_eq!(latest.id, id);
}

#[test]
fn test_snapshot_delete_clears_latest_pointer() {
    let dir = TempDir::new().unwrap();
    let store = SnapshotStore::new(dir.path().to_path_buf());
    let snap1 = Snapshot::new(
        Some("first".into()),
        [1u8; 32],
        None,
        SnapshotStats {
            total_files: 0,
            total_bytes: 0,
            new_objects: 0,
        },
    );
    store.save(&snap1).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(10));
    let snap2 = Snapshot::new(
        Some("second".into()),
        [2u8; 32],
        None,
        SnapshotStats {
            total_files: 0,
            total_bytes: 0,
            new_objects: 0,
        },
    );
    let id1 = snap1.id.clone();
    let id2 = snap2.id.clone();
    store.save(&snap2).unwrap();

    store.delete(&id2).unwrap();

    let latest = store.latest().unwrap().unwrap();
    assert_eq!(latest.id, id1);
}

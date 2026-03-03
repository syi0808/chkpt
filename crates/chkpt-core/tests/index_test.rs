use chkpt_core::index::{FileIndex, FileEntry};
use tempfile::TempDir;

#[test]
fn test_index_insert_and_get() {
    let dir = TempDir::new().unwrap();
    let idx = FileIndex::open(dir.path().join("index.sqlite")).unwrap();
    let entry = FileEntry {
        path: "src/main.rs".into(),
        blob_hash: [1u8; 32],
        size: 100,
        mtime_secs: 1000,
        mtime_nanos: 500,
        inode: Some(12345),
        mode: 0o644,
    };
    idx.upsert(&entry).unwrap();
    let loaded = idx.get("src/main.rs").unwrap().unwrap();
    assert_eq!(loaded.size, 100);
    assert_eq!(loaded.blob_hash, [1u8; 32]);
}

#[test]
fn test_index_get_nonexistent() {
    let dir = TempDir::new().unwrap();
    let idx = FileIndex::open(dir.path().join("index.sqlite")).unwrap();
    assert!(idx.get("nope").unwrap().is_none());
}

#[test]
fn test_index_upsert_updates() {
    let dir = TempDir::new().unwrap();
    let idx = FileIndex::open(dir.path().join("index.sqlite")).unwrap();
    let entry = FileEntry {
        path: "a.txt".into(),
        blob_hash: [0u8; 32],
        size: 10,
        mtime_secs: 100,
        mtime_nanos: 0,
        inode: None,
        mode: 0o644,
    };
    idx.upsert(&entry).unwrap();
    let mut updated = entry.clone();
    updated.size = 20;
    updated.blob_hash = [1u8; 32];
    idx.upsert(&updated).unwrap();
    let loaded = idx.get("a.txt").unwrap().unwrap();
    assert_eq!(loaded.size, 20);
    assert_eq!(loaded.blob_hash, [1u8; 32]);
}

#[test]
fn test_index_remove() {
    let dir = TempDir::new().unwrap();
    let idx = FileIndex::open(dir.path().join("index.sqlite")).unwrap();
    let entry = FileEntry {
        path: "del.txt".into(),
        blob_hash: [0u8; 32],
        size: 5,
        mtime_secs: 50,
        mtime_nanos: 0,
        inode: None,
        mode: 0o644,
    };
    idx.upsert(&entry).unwrap();
    idx.remove("del.txt").unwrap();
    assert!(idx.get("del.txt").unwrap().is_none());
}

#[test]
fn test_index_all_paths() {
    let dir = TempDir::new().unwrap();
    let idx = FileIndex::open(dir.path().join("index.sqlite")).unwrap();
    for name in &["a.txt", "b.txt", "c.txt"] {
        idx.upsert(&FileEntry {
            path: name.to_string(),
            blob_hash: [0u8; 32],
            size: 1,
            mtime_secs: 1,
            mtime_nanos: 0,
            inode: None,
            mode: 0o644,
        }).unwrap();
    }
    let paths = idx.all_paths().unwrap();
    assert_eq!(paths.len(), 3);
}

#[test]
fn test_index_bulk_upsert() {
    let dir = TempDir::new().unwrap();
    let idx = FileIndex::open(dir.path().join("index.sqlite")).unwrap();
    let entries: Vec<FileEntry> = (0..100).map(|i| FileEntry {
        path: format!("file_{}.txt", i),
        blob_hash: [i as u8; 32],
        size: i as u64,
        mtime_secs: 1000 + i as i64,
        mtime_nanos: 0,
        inode: None,
        mode: 0o644,
    }).collect();
    idx.bulk_upsert(&entries).unwrap();
    assert_eq!(idx.all_paths().unwrap().len(), 100);
}

#[test]
fn test_index_clear() {
    let dir = TempDir::new().unwrap();
    let idx = FileIndex::open(dir.path().join("index.sqlite")).unwrap();
    idx.upsert(&FileEntry {
        path: "x.txt".into(), blob_hash: [0u8; 32], size: 1,
        mtime_secs: 1, mtime_nanos: 0, inode: None, mode: 0o644,
    }).unwrap();
    idx.clear().unwrap();
    assert_eq!(idx.all_paths().unwrap().len(), 0);
}

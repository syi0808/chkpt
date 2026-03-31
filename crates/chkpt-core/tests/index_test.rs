use chkpt_core::index::{FileEntry, FileIndex};
use tempfile::TempDir;

fn make_entry(path: &str, hash_byte: u8, size: u64) -> FileEntry {
    FileEntry {
        path: path.to_string(),
        blob_hash: [hash_byte; 32],
        size,
        mtime_secs: 1000 + size as i64,
        mtime_nanos: 0,
        inode: Some(size + 100),
        mode: 0o644,
    }
}

#[test]
fn test_index_open_empty() {
    let dir = TempDir::new().unwrap();
    let idx = FileIndex::open(dir.path().join("index.bin")).unwrap();
    assert_eq!(idx.entries_by_path().unwrap().len(), 0);
}

#[test]
fn test_index_insert_and_get() {
    let dir = TempDir::new().unwrap();
    let mut idx = FileIndex::open(dir.path().join("index.bin")).unwrap();
    let entry = make_entry("src/main.rs", 1, 100);
    idx.upsert(&entry).unwrap();
    let loaded = idx.get("src/main.rs").unwrap().unwrap();
    assert_eq!(loaded.size, 100);
    assert_eq!(loaded.blob_hash, [1u8; 32]);
}

#[test]
fn test_index_get_nonexistent() {
    let dir = TempDir::new().unwrap();
    let idx = FileIndex::open(dir.path().join("index.bin")).unwrap();
    assert!(idx.get("nope").unwrap().is_none());
}

#[test]
fn test_index_upsert_updates() {
    let dir = TempDir::new().unwrap();
    let mut idx = FileIndex::open(dir.path().join("index.bin")).unwrap();
    idx.upsert(&make_entry("a.txt", 0, 10)).unwrap();
    idx.upsert(&make_entry("a.txt", 1, 20)).unwrap();
    let loaded = idx.get("a.txt").unwrap().unwrap();
    assert_eq!(loaded.size, 20);
    assert_eq!(loaded.blob_hash, [1u8; 32]);
}

#[test]
fn test_index_remove() {
    let dir = TempDir::new().unwrap();
    let mut idx = FileIndex::open(dir.path().join("index.bin")).unwrap();
    idx.upsert(&make_entry("del.txt", 0, 5)).unwrap();
    idx.remove("del.txt").unwrap();
    assert!(idx.get("del.txt").unwrap().is_none());
}

#[test]
fn test_index_all_paths() {
    let dir = TempDir::new().unwrap();
    let mut idx = FileIndex::open(dir.path().join("index.bin")).unwrap();
    for (i, name) in ["a.txt", "b.txt", "c.txt"].iter().enumerate() {
        idx.upsert(&make_entry(name, i as u8, 1)).unwrap();
    }
    let paths = idx.all_paths().unwrap();
    assert_eq!(paths.len(), 3);
}

#[test]
fn test_index_bulk_upsert() {
    let dir = TempDir::new().unwrap();
    let mut idx = FileIndex::open(dir.path().join("index.bin")).unwrap();
    let entries: Vec<FileEntry> = (0..100)
        .map(|i| make_entry(&format!("file_{}.txt", i), i as u8, i as u64))
        .collect();
    idx.bulk_upsert(&entries).unwrap();
    assert_eq!(idx.all_paths().unwrap().len(), 100);
}

#[test]
fn test_index_clear() {
    let dir = TempDir::new().unwrap();
    let mut idx = FileIndex::open(dir.path().join("index.bin")).unwrap();
    idx.upsert(&make_entry("x.txt", 0, 1)).unwrap();
    idx.clear().unwrap();
    assert_eq!(idx.all_paths().unwrap().len(), 0);
}

#[test]
fn test_index_entries_by_path() {
    let dir = TempDir::new().unwrap();
    let mut idx = FileIndex::open(dir.path().join("index.bin")).unwrap();
    idx.bulk_upsert(&[make_entry("a.txt", 1, 1), make_entry("b.txt", 2, 2)])
        .unwrap();
    let entries = idx.entries_by_path().unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries["a.txt"].blob_hash, [1u8; 32]);
    assert_eq!(entries["b.txt"].size, 2);
}

#[test]
fn test_index_apply_changes_updates_and_removes_in_one_call() {
    let dir = TempDir::new().unwrap();
    let mut idx = FileIndex::open(dir.path().join("index.bin")).unwrap();
    idx.bulk_upsert(&[make_entry("keep.txt", 1, 1), make_entry("remove.txt", 2, 2)])
        .unwrap();

    idx.apply_changes(
        &[String::from("remove.txt")],
        &[FileEntry {
            path: "keep.txt".into(),
            blob_hash: [3u8; 32],
            size: 3,
            mtime_secs: 3,
            mtime_nanos: 0,
            inode: Some(10),
            mode: 0o755,
        }],
    )
    .unwrap();

    let entries = idx.entries_by_path().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries["keep.txt"].blob_hash, [3u8; 32]);
    assert_eq!(entries["keep.txt"].mode, 0o755);
    assert!(!entries.contains_key("remove.txt"));
}

#[test]
fn test_index_persistence_across_opens() {
    let dir = TempDir::new().unwrap();
    let index_path = dir.path().join("index.bin");

    {
        let mut idx = FileIndex::open(&index_path).unwrap();
        idx.upsert(&make_entry("persist.txt", 42, 999)).unwrap();
    }

    let idx2 = FileIndex::open(&index_path).unwrap();
    let loaded = idx2.get("persist.txt").unwrap().unwrap();
    assert_eq!(loaded.blob_hash, [42u8; 32]);
    assert_eq!(loaded.size, 999);
}

use chkpt_core::store::blob::{hash_content_bytes, hash_path_bytes, BlobStore};
use std::fs;
use tempfile::TempDir;

#[test]
fn test_store_and_read_blob() {
    let dir = TempDir::new().unwrap();
    let store = BlobStore::new(dir.path().to_path_buf());
    let content = b"hello world";
    let hash = store.write(content).unwrap();
    let read_back = store.read(&hash).unwrap();
    assert_eq!(read_back, content);
}

#[test]
fn test_blob_hash_deterministic() {
    let dir = TempDir::new().unwrap();
    let store = BlobStore::new(dir.path().to_path_buf());
    let h1 = store.write(b"same content").unwrap();
    let h2 = store.write(b"same content").unwrap();
    assert_eq!(h1, h2);
}

#[test]
fn test_blob_dedup() {
    let dir = TempDir::new().unwrap();
    let store = BlobStore::new(dir.path().to_path_buf());
    store.write(b"dedup me").unwrap();
    store.write(b"dedup me").unwrap();
    // Only one file should exist (dedup)
    let count: usize = walkdir(dir.path());
    assert_eq!(count, 1);
}

fn walkdir(path: &std::path::Path) -> usize {
    let mut count = 0;
    for entry in std::fs::read_dir(path).unwrap() {
        let entry = entry.unwrap();
        if entry.file_type().unwrap().is_dir() {
            count += walkdir(&entry.path());
        } else {
            count += 1;
        }
    }
    count
}

#[test]
fn test_blob_exists() {
    let dir = TempDir::new().unwrap();
    let store = BlobStore::new(dir.path().to_path_buf());
    let hash = store.write(b"exists").unwrap();
    assert!(store.exists(&hash));
    assert!(!store.exists(&"0".repeat(64)));
}

#[test]
fn test_hash_content_without_storing() {
    let hash = chkpt_core::store::blob::hash_content(b"test");
    assert_eq!(hash.len(), 64); // 64 hex chars
}

#[test]
fn test_hash_path_bytes_matches_hash_content() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("large.txt");
    let content = "stream me ".repeat(4096);
    fs::write(&path, &content).unwrap();

    assert_eq!(
        hash_path_bytes(&path, false).unwrap(),
        hash_content_bytes(content.as_bytes())
    );
}

#[test]
fn test_blob_write_if_missing_with_known_hash() {
    let dir = TempDir::new().unwrap();
    let store = BlobStore::new(dir.path().to_path_buf());
    let content = b"write with known hash";
    let hash = chkpt_core::store::blob::hash_content(content);

    assert!(store.write_if_missing(&hash, content).unwrap());
    let read_back = store.read(&hash).unwrap();

    assert_eq!(read_back, content);
}

#[test]
fn test_blob_write_precompressed_if_missing() {
    let dir = TempDir::new().unwrap();
    let store = BlobStore::new(dir.path().to_path_buf());
    let content = b"precompressed";
    let hash = chkpt_core::store::blob::hash_content(content);
    let compressed = zstd::encode_all(&content[..], 3).unwrap();

    assert!(store
        .write_precompressed_if_missing(&hash, &compressed)
        .unwrap());
    assert_eq!(store.read(&hash).unwrap(), content);
    assert!(!store
        .write_precompressed_if_missing(&hash, &compressed)
        .unwrap());
}

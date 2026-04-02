use chkpt_core::store::blob::{hash_content, hash_content_bytes, hash_path_bytes, read_path_bytes};
use std::fs;
use tempfile::TempDir;

#[test]
fn test_blob_hash_deterministic() {
    let buf = b"same content";
    assert_eq!(hash_content(buf), hash_content(buf));
    assert_eq!(hash_content_bytes(buf), hash_content_bytes(buf));
}

#[test]
fn test_hash_content_without_storing() {
    let hash = hash_content(b"test");
    assert_eq!(hash.len(), 64);
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
fn test_read_path_bytes_reads_regular_files() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("hello.txt");
    fs::write(&path, "hello world").unwrap();

    assert_eq!(read_path_bytes(&path, false).unwrap(), b"hello world");
}

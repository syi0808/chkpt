use chkpt_core::store::blob::{
    bytes_to_hex, hash_content, hash_content_bytes, hash_path_bytes, hex_to_bytes, read_or_mmap,
    read_path_bytes, FileContent,
};
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
    assert_eq!(hash.len(), 32);
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

#[test]
fn test_read_or_mmap_small_file_returns_vec() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("small.txt");
    fs::write(&path, "small content").unwrap();

    let fc = read_or_mmap(&path).unwrap();
    assert!(matches!(fc, FileContent::Vec(_)));
    assert_eq!(fc.as_ref(), b"small content");
}

#[test]
fn test_read_or_mmap_large_file_returns_mmap() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("large.bin");
    // Write > 256 KB to trigger mmap path
    let data = vec![0xABu8; 300 * 1024];
    fs::write(&path, &data).unwrap();

    let fc = read_or_mmap(&path).unwrap();
    assert!(matches!(fc, FileContent::Mmap(_)));
    assert_eq!(fc.as_ref(), data.as_slice());
}

#[test]
fn test_read_or_mmap_empty_file_works() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("empty.txt");
    fs::write(&path, b"").unwrap();

    let fc = read_or_mmap(&path).unwrap();
    assert!(matches!(fc, FileContent::Vec(_)));
    assert_eq!(fc.as_ref(), b"");
}

#[test]
fn test_hex_to_bytes_roundtrip() {
    let original = [
        0xA3u8, 0xB2, 0xC1, 0xD4, 0xE5, 0xF6, 0x07, 0x18, 0x29, 0x3A, 0x4B, 0x5C, 0x6D, 0x7E, 0x8F,
        0x90,
    ];
    let hex = bytes_to_hex(&original);
    let decoded = hex_to_bytes(&hex).unwrap();
    assert_eq!(decoded, original);
}

#[test]
fn test_hex_to_bytes_invalid_length() {
    assert!(hex_to_bytes("abc").is_err());
}

#[test]
fn test_hex_to_bytes_invalid_chars() {
    let bad = "zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz";
    assert!(hex_to_bytes(bad).is_err());
}

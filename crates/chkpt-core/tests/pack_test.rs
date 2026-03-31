use chkpt_core::store::blob::{hash_content, BlobStore};
use chkpt_core::store::pack::{PackReader, PackSet, PackWriter};
use tempfile::TempDir;

#[test]
fn test_pack_write_and_read() {
    let dir = TempDir::new().unwrap();
    let packs_dir = dir.path().join("packs");
    std::fs::create_dir_all(&packs_dir).unwrap();

    let entries: Vec<(String, Vec<u8>)> = vec![
        ("hello".into(), b"hello world".to_vec()),
        ("bye".into(), b"goodbye".to_vec()),
    ];
    let hashes: Vec<String> = entries.iter().map(|(_, data)| hash_content(data)).collect();

    let mut writer = PackWriter::new(&packs_dir).unwrap();
    for (_, data) in &entries {
        writer.add(data).unwrap();
    }
    let pack_hash = writer.finish().unwrap();

    let reader = PackReader::open(&packs_dir, &pack_hash).unwrap();
    let data0 = reader.read(&hashes[0]).unwrap();
    assert_eq!(data0, b"hello world");
    let data1 = reader.read(&hashes[1]).unwrap();
    assert_eq!(data1, b"goodbye");
}

#[test]
fn test_pack_index_binary_search() {
    let dir = TempDir::new().unwrap();
    let packs_dir = dir.path().join("packs");
    std::fs::create_dir_all(&packs_dir).unwrap();

    let mut writer = PackWriter::new(&packs_dir).unwrap();
    for i in 0..100 {
        let data = format!("content-{}", i);
        writer.add(data.as_bytes()).unwrap();
    }
    let pack_hash = writer.finish().unwrap();

    let reader = PackReader::open(&packs_dir, &pack_hash).unwrap();
    let target = hash_content(b"content-50");
    let data = reader.read(&target).unwrap();
    assert_eq!(data, b"content-50");
}

#[test]
fn test_pack_not_found_returns_none() {
    let dir = TempDir::new().unwrap();
    let packs_dir = dir.path().join("packs");
    std::fs::create_dir_all(&packs_dir).unwrap();

    let mut writer = PackWriter::new(&packs_dir).unwrap();
    writer.add(b"data").unwrap();
    let pack_hash = writer.finish().unwrap();

    let reader = PackReader::open(&packs_dir, &pack_hash).unwrap();
    let result = reader.try_read(&"0".repeat(64));
    assert!(result.is_none());
}

#[test]
fn test_pack_from_loose_objects() {
    let dir = TempDir::new().unwrap();
    let objects_dir = dir.path().join("objects");
    let packs_dir = dir.path().join("packs");
    std::fs::create_dir_all(&objects_dir).unwrap();
    std::fs::create_dir_all(&packs_dir).unwrap();

    let blob_store = BlobStore::new(objects_dir.clone());
    let mut hashes = Vec::new();
    for i in 0..10 {
        let h = blob_store.write(format!("file-{}", i).as_bytes()).unwrap();
        hashes.push(h);
    }

    // Pack all loose objects
    let pack_hash = chkpt_core::store::pack::pack_loose_objects(&blob_store, &packs_dir).unwrap();

    // Loose objects should be deleted
    assert_eq!(blob_store.list_loose().unwrap().len(), 0);

    // All data should be readable from pack
    let reader = PackReader::open(&packs_dir, &pack_hash).unwrap();
    for (i, hash) in hashes.iter().enumerate() {
        let data = reader.read(hash).unwrap();
        assert_eq!(data, format!("file-{}", i).as_bytes());
    }
}

#[test]
fn test_pack_set_reads_across_multiple_packs() {
    let dir = TempDir::new().unwrap();
    let packs_dir = dir.path().join("packs");
    std::fs::create_dir_all(&packs_dir).unwrap();

    let mut writer_one = PackWriter::new(&packs_dir).unwrap();
    let hash_one = writer_one.add(b"first-pack").unwrap();
    writer_one.finish().unwrap();

    let mut writer_two = PackWriter::new(&packs_dir).unwrap();
    let hash_two = writer_two.add(b"second-pack").unwrap();
    writer_two.finish().unwrap();

    let pack_set = PackSet::open_all(&packs_dir).unwrap();

    assert_eq!(pack_set.read(&hash_one).unwrap(), b"first-pack");
    assert_eq!(pack_set.read(&hash_two).unwrap(), b"second-pack");
}

#[test]
fn test_pack_write_with_precompressed_entries() {
    let dir = TempDir::new().unwrap();
    let packs_dir = dir.path().join("packs");
    std::fs::create_dir_all(&packs_dir).unwrap();

    let content = b"streamed-content".to_vec();
    let hash = hash_content(&content);
    let compressed = zstd::encode_all(&content[..], 3).unwrap();

    let mut writer = PackWriter::new(&packs_dir).unwrap();
    writer.add_pre_compressed(hash.clone(), compressed).unwrap();
    let pack_hash = writer.finish().unwrap();

    let reader = PackReader::open(&packs_dir, &pack_hash).unwrap();
    assert_eq!(reader.read(&hash).unwrap(), content);
}

#[test]
fn test_pack_streaming_write_and_read() {
    let dir = TempDir::new().unwrap();
    let packs_dir = dir.path().join("packs");
    std::fs::create_dir_all(&packs_dir).unwrap();

    // Write 1000 entries via streaming PackWriter
    let mut writer = PackWriter::new(&packs_dir).unwrap();
    let mut expected: Vec<(String, Vec<u8>)> = Vec::with_capacity(1000);
    for i in 0..1000 {
        let data = format!("streaming-entry-{}", i);
        let hash = writer.add(data.as_bytes()).unwrap();
        expected.push((hash, data.into_bytes()));
    }
    assert!(!writer.is_empty());
    let pack_hash = writer.finish().unwrap();

    // Read back all 1000 entries and verify
    let reader = PackReader::open(&packs_dir, &pack_hash).unwrap();
    for (hash, data) in &expected {
        let read_data = reader.read(hash).unwrap();
        assert_eq!(&read_data, data);
    }

    // Verify hashes list contains all entries
    let all_hashes = reader.hashes();
    assert_eq!(all_hashes.len(), 1000);
    for (hash, _) in &expected {
        assert!(all_hashes.contains(hash));
    }
}

#[test]
fn test_pack_mmap_reader_large_dataset() {
    let dir = TempDir::new().unwrap();
    let packs_dir = dir.path().join("packs");
    std::fs::create_dir_all(&packs_dir).unwrap();

    let mut writer = PackWriter::new(&packs_dir).unwrap();
    let mut expected: Vec<(String, Vec<u8>)> = Vec::with_capacity(1000);
    for i in 0..1000 {
        let data = format!("mmap-large-dataset-entry-{:04}", i);
        let hash = writer.add(data.as_bytes()).unwrap();
        expected.push((hash, data.into_bytes()));
    }
    let pack_hash = writer.finish().unwrap();

    // Open with mmap-backed reader
    let reader = PackReader::open(&packs_dir, &pack_hash).unwrap();

    // Verify every entry can be read back correctly
    for (hash, data) in &expected {
        assert!(reader.contains(hash));
        let read_data = reader.read(hash).unwrap();
        assert_eq!(&read_data, data);
    }

    // Verify non-existent hash returns None
    let fake_hash = "0".repeat(64);
    assert!(!reader.contains(&fake_hash));
    assert!(reader.try_read(&fake_hash).is_none());

    // Verify hashes list is complete
    let all_hashes = reader.hashes();
    assert_eq!(all_hashes.len(), 1000);
    for (hash, _) in &expected {
        assert!(all_hashes.contains(hash));
    }
}

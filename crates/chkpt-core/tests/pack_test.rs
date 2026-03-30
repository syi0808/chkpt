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

    let mut writer = PackWriter::new();
    for (_, data) in &entries {
        writer.add(data).unwrap();
    }
    let pack_hash = writer.finish(&packs_dir).unwrap();

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

    let mut writer = PackWriter::new();
    for i in 0..100 {
        let data = format!("content-{}", i);
        writer.add(data.as_bytes()).unwrap();
    }
    let pack_hash = writer.finish(&packs_dir).unwrap();

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

    let mut writer = PackWriter::new();
    writer.add(b"data").unwrap();
    let pack_hash = writer.finish(&packs_dir).unwrap();

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

    let mut writer_one = PackWriter::new();
    let hash_one = writer_one.add(b"first-pack").unwrap();
    writer_one.finish(&packs_dir).unwrap();

    let mut writer_two = PackWriter::new();
    let hash_two = writer_two.add(b"second-pack").unwrap();
    writer_two.finish(&packs_dir).unwrap();

    let pack_set = PackSet::open_all(&packs_dir).unwrap();

    assert_eq!(pack_set.read(&hash_one).unwrap(), b"first-pack");
    assert_eq!(pack_set.read(&hash_two).unwrap(), b"second-pack");
}

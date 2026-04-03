use chkpt_core::store::blob::hash_content;
use chkpt_core::store::blob::hash_content_bytes;
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
    let result = reader.try_read(&"0".repeat(32));
    assert!(result.is_none());
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
fn test_pack_set_open_selected_limits_visible_packs() {
    let dir = TempDir::new().unwrap();
    let packs_dir = dir.path().join("packs");
    std::fs::create_dir_all(&packs_dir).unwrap();

    let mut writer_one = PackWriter::new(&packs_dir).unwrap();
    let hash_one = writer_one.add(b"first-pack").unwrap();
    let pack_one = writer_one.finish().unwrap();

    let mut writer_two = PackWriter::new(&packs_dir).unwrap();
    let hash_two = writer_two.add(b"second-pack").unwrap();
    writer_two.finish().unwrap();

    let pack_set = PackSet::open_selected(&packs_dir, &[pack_one]).unwrap();

    assert_eq!(pack_set.read(&hash_one).unwrap(), b"first-pack");
    assert!(pack_set.try_read(&hash_two).is_none());
}

#[test]
fn test_pack_write_with_precompressed_entries() {
    let dir = TempDir::new().unwrap();
    let packs_dir = dir.path().join("packs");
    std::fs::create_dir_all(&packs_dir).unwrap();

    let content = b"streamed-content".to_vec();
    let hash = hash_content(&content);
    let compressed = {
        use lz4_flex::frame::FrameEncoder;
        let mut enc = FrameEncoder::new(Vec::new());
        std::io::Write::write_all(&mut enc, &content).unwrap();
        enc.finish().unwrap()
    };

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
        let read_data = reader.read(hash).unwrap();
        assert_eq!(&read_data, data);
    }

    // Verify non-existent hash returns None
    let fake_hash = "0".repeat(32);
    assert!(reader.try_read(&fake_hash).is_none());
}

#[test]
fn test_pack_locate_bytes() {
    let dir = TempDir::new().unwrap();
    let packs_dir = dir.path().join("packs");
    std::fs::create_dir_all(&packs_dir).unwrap();

    let content = b"locate-bytes-test-data";
    let hash_bytes = hash_content_bytes(content);

    let mut writer = PackWriter::new(&packs_dir).unwrap();
    writer.add(content).unwrap();
    writer.finish().unwrap();

    let pack_set = PackSet::open_all(&packs_dir).unwrap();
    let location = pack_set.locate_bytes(&hash_bytes);
    assert!(location.is_some());

    let missing = [0u8; 16];
    assert!(pack_set.locate_bytes(&missing).is_none());
}

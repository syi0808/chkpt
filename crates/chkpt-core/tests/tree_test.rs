use chkpt_core::store::tree::{EntryType, TreeEntry, TreeStore};
use tempfile::TempDir;

#[test]
fn test_tree_roundtrip() {
    let dir = TempDir::new().unwrap();
    let store = TreeStore::new(dir.path().to_path_buf());
    let entries = vec![
        TreeEntry {
            name: "bar.txt".into(),
            entry_type: EntryType::File,
            hash: [1u8; 16],
            size: 100,
            mode: 0o644,
        },
        TreeEntry {
            name: "foo.txt".into(),
            entry_type: EntryType::File,
            hash: [2u8; 16],
            size: 200,
            mode: 0o644,
        },
    ];
    let hash = store.write(&entries).unwrap();
    let read_back = store.read(&hash).unwrap();
    assert_eq!(read_back.len(), 2);
    assert_eq!(read_back[0].name, "bar.txt"); // sorted
}

#[test]
fn test_tree_hash_deterministic() {
    let dir = TempDir::new().unwrap();
    let store = TreeStore::new(dir.path().to_path_buf());
    let entries = vec![TreeEntry {
        name: "a.txt".into(),
        entry_type: EntryType::File,
        hash: [0u8; 16],
        size: 10,
        mode: 0o644,
    }];
    let h1 = store.write(&entries).unwrap();
    let h2 = store.write(&entries).unwrap();
    assert_eq!(h1, h2);
}

#[test]
fn test_tree_sorts_entries() {
    let dir = TempDir::new().unwrap();
    let store = TreeStore::new(dir.path().to_path_buf());
    let entries = vec![
        TreeEntry {
            name: "z".into(),
            entry_type: EntryType::File,
            hash: [0u8; 16],
            size: 0,
            mode: 0o644,
        },
        TreeEntry {
            name: "a".into(),
            entry_type: EntryType::File,
            hash: [1u8; 16],
            size: 0,
            mode: 0o644,
        },
    ];
    let hash = store.write(&entries).unwrap();
    let read_back = store.read(&hash).unwrap();
    assert_eq!(read_back[0].name, "a");
    assert_eq!(read_back[1].name, "z");
}

#[test]
fn test_tree_with_dir_entry() {
    let dir = TempDir::new().unwrap();
    let store = TreeStore::new(dir.path().to_path_buf());
    let entries = vec![
        TreeEntry {
            name: "src".into(),
            entry_type: EntryType::Dir,
            hash: [5u8; 16],
            size: 0,
            mode: 0o755,
        },
        TreeEntry {
            name: "README.md".into(),
            entry_type: EntryType::File,
            hash: [6u8; 16],
            size: 50,
            mode: 0o644,
        },
    ];
    let hash = store.write(&entries).unwrap();
    let read_back = store.read(&hash).unwrap();
    assert_eq!(read_back.len(), 2);
}

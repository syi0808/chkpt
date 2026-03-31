use crate::config::{project_id_from_path, StoreLayout};
use crate::error::{ChkpttError, Result};
use crate::index::{FileEntry, FileIndex};
use crate::ops::lock::ProjectLock;
use crate::scanner::ScannedFile;
use crate::store::blob::BlobStore;
use crate::store::pack::{PackSet, PackWriter};
use crate::store::snapshot::{Snapshot, SnapshotAttachments, SnapshotStats, SnapshotStore};
use crate::store::tree::{EntryType, TreeEntry, TreeStore};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::io::{BufReader, Read, Write};
use std::path::Path;
use std::sync::mpsc;

#[derive(Debug, Default)]
pub struct SaveOptions {
    pub message: Option<String>,
    pub include_deps: bool,
}

#[derive(Debug)]
pub struct SaveResult {
    pub snapshot_id: String,
    pub stats: SnapshotStats,
}

/// Convert a 64-char hex string to a [u8; 32] array.
fn hex_to_bytes(hex: &str) -> Result<[u8; 32]> {
    let mut bytes = [0u8; 32];
    if hex.len() != 64 {
        return Err(ChkpttError::Other(format!(
            "Invalid hash length: {}",
            hex.len()
        )));
    }
    for i in 0..32 {
        bytes[i] = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16)
            .map_err(|_| ChkpttError::Other("Invalid hex".into()))?;
    }
    Ok(bytes)
}

/// Represents a file with its blob hash after processing.
struct ProcessedFile {
    relative_path: String,
    blob_hash_bytes: [u8; 32],
    size: u64,
    mode: u32,
}

struct PreparedFile {
    relative_path: String,
    blob_hash_hex: String,
    blob_hash_bytes: [u8; 32],
    compressed: Vec<u8>,
    size: u64,
    mode: u32,
    mtime_secs: i64,
    mtime_nanos: i64,
    inode: Option<u64>,
}

/// Save a checkpoint of the workspace.
///
/// This is the main orchestrating function that ties together scanning,
/// hashing, blob storage, tree building, and snapshot creation.
pub fn save(workspace_root: &Path, options: SaveOptions) -> Result<SaveResult> {
    // 1. Compute project_id from workspace path
    let project_id = project_id_from_path(workspace_root);

    // 2. Create StoreLayout, ensure directories exist
    let layout = StoreLayout::new(&project_id);
    layout.ensure_dirs()?;

    // 3. Acquire project lock
    let _lock = ProjectLock::acquire(&layout.locks_dir())?;

    // 4. Scan workspace (respect .chkptignore)
    let scanned_files =
        crate::scanner::scan_workspace_with_options(workspace_root, None, options.include_deps)?;

    // 5. Open/create FileIndex
    let mut index = FileIndex::open(layout.index_path())?;
    let cached_entries = index.entries_by_path()?;

    // 6. Create blob store
    let objects_dir = layout.objects_dir();
    let blob_store = BlobStore::new(objects_dir.clone());
    let packs_dir = layout.packs_dir();
    let has_loose_objects = store_has_loose_objects(&objects_dir)?;
    let has_pack_objects = store_has_pack_objects(&packs_dir)?;
    let pack_set = has_pack_objects
        .then(|| PackSet::open_all(&packs_dir))
        .transpose()?;
    let mut pack_writer = PackWriter::new(&packs_dir)?;
    let mut staged_pack_hashes = HashSet::new();

    // 7. Process each scanned file: check index, hash, store blob
    let mut processed_files = Vec::with_capacity(scanned_files.len());
    let mut files_to_prepare = Vec::new();
    let mut updated_entries = Vec::new();
    let mut total_bytes: u64 = 0;
    let mut current_paths =
        (!cached_entries.is_empty()).then(|| HashSet::with_capacity(scanned_files.len()));

    for scanned in &scanned_files {
        if let Some(paths) = current_paths.as_mut() {
            paths.insert(scanned.relative_path.clone());
        }

        if let Some(processed) =
            cached_processed_file(scanned, cached_entries.get(&scanned.relative_path))
        {
            total_bytes += processed.size;
            processed_files.push(processed);
        } else {
            files_to_prepare.push(scanned);
        }
    }
    let removed_paths = current_paths
        .map(|paths| {
            cached_entries
                .keys()
                .filter(|path| !paths.contains(*path))
                .cloned()
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let mut new_objects: u64 = 0;

    // Pipeline: worker threads compress files and send through a bounded channel,
    // main thread receives and writes to pack immediately. This overlaps compression
    // with disk I/O and limits peak memory to channel_bound * avg_compressed_size.
    let channel_bound = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
        * 2;

    for prepared in prepare_files_pipeline(files_to_prepare, channel_bound)? {
        let prepared = prepared?;
        let PreparedFile {
            relative_path,
            blob_hash_hex,
            blob_hash_bytes,
            compressed,
            size,
            mode,
            mtime_secs,
            mtime_nanos,
            inode,
        } = prepared;

        total_bytes += size;
        let exists_externally = (has_loose_objects && blob_store.exists(&blob_hash_hex))
            || pack_set
                .as_ref()
                .is_some_and(|pack_set| pack_set.contains(&blob_hash_hex));
        if !staged_pack_hashes.contains(&blob_hash_hex) && !exists_externally {
            pack_writer.add_pre_compressed(blob_hash_hex.clone(), compressed)?;
            staged_pack_hashes.insert(blob_hash_hex.clone());
            new_objects += 1;
        }

        updated_entries.push(FileEntry {
            path: relative_path.clone(),
            blob_hash: blob_hash_bytes,
            size,
            mtime_secs,
            mtime_nanos,
            inode,
            mode,
        });
        processed_files.push(ProcessedFile {
            relative_path,
            blob_hash_bytes,
            size,
            mode,
        });
    }
    if !pack_writer.is_empty() {
        pack_writer.finish()?;
    }

    // 8. Build tree bottom-up
    let tree_store = TreeStore::new(layout.trees_dir());
    let root_tree_hash_hex = build_tree(&processed_files, &tree_store)?;
    let root_tree_hash = hex_to_bytes(&root_tree_hash_hex)?;

    // 9. Find latest snapshot for parent_snapshot_id
    let snapshot_store = SnapshotStore::new(layout.snapshots_dir());
    let parent_snapshot_id = snapshot_store.latest()?.map(|s| s.id);

    // 10. Create Snapshot
    let stats = SnapshotStats {
        total_files: scanned_files.len() as u64,
        total_bytes,
        new_objects,
    };

    let snapshot = Snapshot::new(
        options.message,
        root_tree_hash,
        parent_snapshot_id,
        SnapshotAttachments::default(),
        stats.clone(),
    );

    let snapshot_id = snapshot.id.clone();

    // 11. Save snapshot
    snapshot_store.save(&snapshot)?;

    // 12. Update only changed index entries and remove stale paths.
    index.apply_changes(&removed_paths, &updated_entries)?;

    // 13. Lock released automatically via drop

    // 14. Return SaveResult
    Ok(SaveResult { snapshot_id, stats })
}

#[cfg_attr(not(test), allow(dead_code))]
fn store_has_external_objects(objects_dir: &Path, packs_dir: &Path) -> Result<bool> {
    Ok(store_has_loose_objects(objects_dir)? || store_has_pack_objects(packs_dir)?)
}

fn store_has_loose_objects(objects_dir: &Path) -> Result<bool> {
    if !objects_dir.exists() {
        return Ok(false);
    }

    for prefix_entry in std::fs::read_dir(objects_dir)? {
        let prefix_entry = prefix_entry?;
        if !prefix_entry.file_type()?.is_dir() {
            continue;
        }

        for object_entry in std::fs::read_dir(prefix_entry.path())? {
            let object_entry = object_entry?;
            if object_entry.file_type()?.is_file()
                && !object_entry.file_name().to_string_lossy().ends_with(".tmp")
            {
                return Ok(true);
            }
        }
    }

    Ok(false)
}

fn store_has_pack_objects(packs_dir: &Path) -> Result<bool> {
    if !packs_dir.exists() {
        return Ok(false);
    }

    for entry in std::fs::read_dir(packs_dir)? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }

        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with("pack-") && name.ends_with(".dat") {
            return Ok(true);
        }
    }

    Ok(false)
}

fn cached_processed_file(
    scanned: &ScannedFile,
    cached: Option<&FileEntry>,
) -> Option<ProcessedFile> {
    if let Some(cached) = cached {
        if cached.mtime_secs == scanned.mtime_secs
            && cached.mtime_nanos == scanned.mtime_nanos
            && cached.size == scanned.size
            && cached.inode == scanned.inode
        {
            return Some(ProcessedFile {
                relative_path: scanned.relative_path.clone(),
                blob_hash_bytes: cached.blob_hash,
                size: scanned.size,
                mode: scanned.mode,
            });
        }
    }
    None
}

fn prepare_file(scanned: &ScannedFile) -> Result<PreparedFile> {
    let file = std::fs::File::open(&scanned.absolute_path)?;
    let mut reader = BufReader::new(file);
    let mut hasher = blake3::Hasher::new();
    let mut encoder = zstd::stream::write::Encoder::new(Vec::new(), 3)?;
    let mut buffer = [0u8; 64 * 1024];

    loop {
        let bytes_read = reader.read(&mut buffer)?;
        if bytes_read == 0 {
            break;
        }
        let chunk = &buffer[..bytes_read];
        hasher.update(chunk);
        encoder.write_all(chunk)?;
    }

    let compressed = encoder.finish()?;
    let blob_hash_hex = hasher.finalize().to_hex().to_string();
    let blob_hash_bytes = hex_to_bytes(&blob_hash_hex)?;

    Ok(PreparedFile {
        relative_path: scanned.relative_path.clone(),
        blob_hash_hex,
        blob_hash_bytes,
        compressed,
        size: scanned.size,
        mode: scanned.mode,
        mtime_secs: scanned.mtime_secs,
        mtime_nanos: scanned.mtime_nanos,
        inode: scanned.inode,
    })
}

/// Prepare files using a producer-consumer pipeline.
///
/// Worker threads compress files and send results through a bounded channel.
/// The caller iterates over the receiver, consuming results as they arrive.
/// This overlaps compression (CPU) with pack writing (I/O).
fn prepare_files_pipeline(
    scanned_files: Vec<&ScannedFile>,
    channel_bound: usize,
) -> Result<mpsc::IntoIter<Result<PreparedFile>>> {
    if scanned_files.is_empty() {
        let (tx, rx) = mpsc::sync_channel(0);
        drop(tx);
        return Ok(rx.into_iter());
    }

    let (tx, rx) = mpsc::sync_channel::<Result<PreparedFile>>(channel_bound);

    // Owned copies for the spawned thread (can't send references across spawn boundary)
    let owned_files: Vec<ScannedFile> = scanned_files.into_iter().cloned().collect();

    std::thread::spawn(move || {
        let worker_count = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1)
            .min(owned_files.len());

        if worker_count <= 1 {
            for scanned in &owned_files {
                if tx.send(prepare_file(scanned)).is_err() {
                    return;
                }
            }
            return;
        }

        let chunk_size = owned_files.len().div_ceil(worker_count);
        std::thread::scope(|scope| {
            for chunk in owned_files.chunks(chunk_size) {
                let tx = tx.clone();
                scope.spawn(move || {
                    for scanned in chunk {
                        if tx.send(prepare_file(scanned)).is_err() {
                            return;
                        }
                    }
                });
            }
        });
    });

    Ok(rx.into_iter())
}

/// Build tree structure bottom-up from processed files.
///
/// Groups files by their directory path, creates tree entries for each file,
/// recursively builds subtrees for subdirectories, and returns the root tree hash.
fn build_tree(processed_files: &[ProcessedFile], tree_store: &TreeStore) -> Result<String> {
    // Group files by parent directory
    let mut dir_files: BTreeMap<String, Vec<&ProcessedFile>> = BTreeMap::new();
    let mut all_dirs: BTreeSet<String> = BTreeSet::new();
    let mut child_dirs: BTreeMap<String, Vec<String>> = BTreeMap::new();
    all_dirs.insert(String::new()); // root always exists

    for pf in processed_files {
        let parent = if let Some(pos) = pf.relative_path.rfind('/') {
            pf.relative_path[..pos].to_string()
        } else {
            String::new() // root directory
        };
        dir_files.entry(parent.clone()).or_default().push(pf);
        register_directory_hierarchy(&parent, &mut all_dirs, &mut child_dirs);
    }

    // Build trees bottom-up: process deepest directories first
    let mut dir_list: Vec<String> = all_dirs.into_iter().collect();
    // Sort by depth (deepest first), then alphabetically for same depth
    dir_list.sort_by(|a, b| {
        let depth_a = if a.is_empty() {
            0
        } else {
            a.matches('/').count() + 1
        };
        let depth_b = if b.is_empty() {
            0
        } else {
            b.matches('/').count() + 1
        };
        depth_b.cmp(&depth_a).then_with(|| a.cmp(b))
    });

    // Map from directory path to its tree hash
    let mut dir_hashes: BTreeMap<String, String> = BTreeMap::new();

    for dir in &dir_list {
        let mut entries: Vec<TreeEntry> = Vec::new();

        // Add file entries for this directory
        if let Some(files) = dir_files.get(dir) {
            for pf in files {
                let name = if let Some(pos) = pf.relative_path.rfind('/') {
                    pf.relative_path[pos + 1..].to_string()
                } else {
                    pf.relative_path.clone()
                };
                entries.push(TreeEntry {
                    name,
                    entry_type: EntryType::File,
                    hash: pf.blob_hash_bytes,
                    size: pf.size,
                    mode: pf.mode,
                });
            }
        }

        // Add subdirectory entries (directories whose parent is this directory)
        if let Some(children) = child_dirs.get(dir) {
            for sub_dir in children {
                let sub_hash = dir_hashes.get(sub_dir).ok_or_else(|| {
                    ChkpttError::Other(format!("Missing tree hash for directory '{}'", sub_dir))
                })?;
                let sub_name = if let Some(pos) = sub_dir.rfind('/') {
                    sub_dir[pos + 1..].to_string()
                } else {
                    sub_dir.clone()
                };
                entries.push(TreeEntry {
                    name: sub_name,
                    entry_type: EntryType::Dir,
                    hash: hex_to_bytes(sub_hash)?,
                    size: 0,
                    mode: 0o040755,
                });
            }
        }

        // Write tree and store hash
        let tree_hash = tree_store.write(&entries)?;
        dir_hashes.insert(dir.clone(), tree_hash);
    }

    // Return root tree hash
    dir_hashes
        .get("")
        .cloned()
        .ok_or_else(|| ChkpttError::Other("Failed to build root tree".into()))
}

fn register_directory_hierarchy(
    dir: &str,
    all_dirs: &mut BTreeSet<String>,
    child_dirs: &mut BTreeMap<String, Vec<String>>,
) {
    if dir.is_empty() {
        return;
    }

    let mut parent = String::new();
    for segment in dir.split('/') {
        let current = if parent.is_empty() {
            segment.to_string()
        } else {
            format!("{}/{}", parent, segment)
        };
        if all_dirs.insert(current.clone()) {
            child_dirs
                .entry(parent.clone())
                .or_default()
                .push(current.clone());
        }
        parent = current;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_hex_to_bytes() {
        let hex = "a3b2c1d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f90";
        let bytes = hex_to_bytes(hex).unwrap();
        assert_eq!(bytes[0], 0xa3);
        assert_eq!(bytes[1], 0xb2);
        assert_eq!(bytes[31], 0x90);
    }

    #[test]
    fn test_hex_to_bytes_invalid_length() {
        assert!(hex_to_bytes("abc").is_err());
    }

    #[test]
    fn test_store_has_external_objects_false_for_empty_store() {
        let dir = TempDir::new().unwrap();
        let objects_dir = dir.path().join("objects");
        let packs_dir = dir.path().join("packs");
        std::fs::create_dir_all(&objects_dir).unwrap();
        std::fs::create_dir_all(&packs_dir).unwrap();

        assert!(!store_has_external_objects(&objects_dir, &packs_dir).unwrap());
    }

    #[test]
    fn test_store_has_external_objects_true_for_loose_or_pack_objects() {
        let dir = TempDir::new().unwrap();
        let objects_dir = dir.path().join("objects");
        let packs_dir = dir.path().join("packs");
        std::fs::create_dir_all(objects_dir.join("aa")).unwrap();
        std::fs::create_dir_all(&packs_dir).unwrap();
        std::fs::write(objects_dir.join("aa").join("bb"), b"compressed").unwrap();

        assert!(store_has_external_objects(&objects_dir, &packs_dir).unwrap());

        std::fs::remove_file(objects_dir.join("aa").join("bb")).unwrap();
        std::fs::write(packs_dir.join("pack-demo.dat"), b"pack").unwrap();

        assert!(store_has_external_objects(&objects_dir, &packs_dir).unwrap());
    }
}

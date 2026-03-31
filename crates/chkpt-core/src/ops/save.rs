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
use std::path::Path;
use std::sync::{Arc, Mutex};

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
    /// None if this hash was already seen (duplicate content — compression skipped)
    compressed: Option<Vec<u8>>,
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
    let staged_pack_hashes: HashSet<[u8; 32]> = HashSet::new();

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

    // Shared dedup set: workers check before compressing to skip duplicate content
    let seen_hashes = Arc::new(Mutex::new(staged_pack_hashes));

    // Batch parallel: each thread processes a chunk independently, no channel overhead
    let prepared_results = prepare_files_batch(&files_to_prepare, &seen_hashes)?;

    for prepared in prepared_results {
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
        if let Some(compressed) = compressed {
            let exists_externally = (has_loose_objects && blob_store.exists(&blob_hash_hex))
                || pack_set
                    .as_ref()
                    .is_some_and(|ps| ps.contains(&blob_hash_hex));
            if !exists_externally {
                pack_writer.add_pre_compressed(blob_hash_hex, compressed)?;
                new_objects += 1;
            }
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

    // 8. Build tree bottom-up (skip if nothing changed)
    let snapshot_store = SnapshotStore::new(layout.snapshots_dir());
    let latest_snapshot = snapshot_store.latest()?;

    let (root_tree_hash, _root_tree_hash_hex) =
        if new_objects == 0 && removed_paths.is_empty() {
            if let Some(ref snap) = latest_snapshot {
                // Nothing changed — reuse previous tree hash (skip build entirely)
                (snap.root_tree_hash, String::new())
            } else {
                let tree_store = TreeStore::new(layout.trees_dir());
                let hex = build_tree(&processed_files, &tree_store)?;
                (hex_to_bytes(&hex)?, hex)
            }
        } else {
            let tree_store = TreeStore::new(layout.trees_dir());
            let hex = build_tree(&processed_files, &tree_store)?;
            (hex_to_bytes(&hex)?, hex)
        };

    // 9. Find latest snapshot for parent_snapshot_id (already fetched above)
    let parent_snapshot_id = latest_snapshot.map(|s| s.id);

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

fn prepare_file(
    scanned: &ScannedFile,
    seen_hashes: &Mutex<HashSet<[u8; 32]>>,
) -> Result<PreparedFile> {
    // Read file with pre-allocated buffer (skip fstat overhead of fs::read)
    let content = {
        use std::io::Read;
        let mut file = std::fs::File::open(&scanned.absolute_path)?;
        let mut buf = Vec::with_capacity(scanned.size as usize);
        file.read_to_end(&mut buf)?;
        buf
    };

    // Hash (BLAKE3 is ~6 GB/s, negligible cost)
    let hash = blake3::hash(&content);
    let blob_hash_bytes: [u8; 32] = *hash.as_bytes();

    // Only compress if this hash hasn't been seen yet (use raw bytes, no hex conversion)
    let is_new = {
        let mut set = seen_hashes.lock().unwrap();
        set.insert(blob_hash_bytes)
    };

    let compressed = if is_new {
        Some(zstd::encode_all(&content[..], 1)?)
    } else {
        None
    };

    // Defer hex conversion to caller (only needed for pack writer)
    let blob_hash_hex = hash.to_hex().to_string();

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

/// Prepare files in parallel batches.
/// Each thread processes a chunk independently and collects results in a local Vec.
/// No channel overhead — threads run at full speed.
fn prepare_files_batch(
    scanned_files: &[&ScannedFile],
    seen_hashes: &Arc<Mutex<HashSet<[u8; 32]>>>,
) -> Result<Vec<PreparedFile>> {
    if scanned_files.is_empty() {
        return Ok(Vec::new());
    }

    let worker_count = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
        .min(scanned_files.len());

    if worker_count <= 1 {
        return scanned_files
            .iter()
            .map(|s| prepare_file(s, seen_hashes))
            .collect();
    }

    let chunk_size = scanned_files.len().div_ceil(worker_count);

    let results: Vec<Vec<Result<PreparedFile>>> = std::thread::scope(|scope| {
        let handles: Vec<_> = scanned_files
            .chunks(chunk_size)
            .map(|chunk| {
                scope.spawn(|| {
                    chunk
                        .iter()
                        .map(|scanned| prepare_file(scanned, seen_hashes))
                        .collect::<Vec<_>>()
                })
            })
            .collect();

        handles
            .into_iter()
            .map(|h| h.join().unwrap())
            .collect()
    });

    // Flatten and propagate errors
    let mut all = Vec::with_capacity(scanned_files.len());
    for chunk_results in results {
        for result in chunk_results {
            all.push(result?);
        }
    }
    Ok(all)
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
            String::new()
        };
        dir_files.entry(parent.clone()).or_default().push(pf);
        register_directory_hierarchy(&parent, &mut all_dirs, &mut child_dirs);
    }

    // Sort directories bottom-up (deepest first)
    let mut dir_list: Vec<String> = all_dirs.into_iter().collect();
    dir_list.sort_unstable_by(|a, b| {
        let depth_a = if a.is_empty() { 0 } else { a.matches('/').count() + 1 };
        let depth_b = if b.is_empty() { 0 } else { b.matches('/').count() + 1 };
        depth_b.cmp(&depth_a).then_with(|| a.cmp(b))
    });

    // Phase 1: Compute all tree hashes and encoded data in memory
    let mut dir_hashes: BTreeMap<String, String> = BTreeMap::new();
    let mut pack_entries: Vec<(String, Vec<u8>)> = Vec::with_capacity(dir_list.len());
    let mut known_hashes: HashSet<String> = HashSet::with_capacity(dir_list.len());

    for dir in &dir_list {
        let mut entries: Vec<TreeEntry> = Vec::new();

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

        entries.sort_unstable_by(|a, b| a.name.cmp(&b.name));
        let encoded = bitcode::encode(&entries);
        let hash_hex = blake3::hash(&encoded).to_hex().to_string();

        dir_hashes.insert(dir.clone(), hash_hex.clone());
        if known_hashes.insert(hash_hex.clone()) {
            pack_entries.push((hash_hex, encoded));
        }
    }

    // Phase 2: Write all trees to a single pack file (1 write instead of 37K+)
    tree_store.write_pack(&pack_entries)?;

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

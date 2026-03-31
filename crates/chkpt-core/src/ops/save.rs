use crate::config::{project_id_from_path, StoreLayout};
use crate::error::{ChkpttError, Result};
use crate::index::{FileEntry, FileIndex};
use crate::ops::lock::ProjectLock;
use crate::scanner::{scan_workspace, ScannedFile};
use crate::store::blob::BlobStore;
use crate::store::pack::{PackSet, PackWriter};
use crate::store::snapshot::{Snapshot, SnapshotAttachments, SnapshotStats, SnapshotStore};
use crate::store::tree::{EntryType, TreeEntry, TreeStore};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::io::{BufReader, Read, Write};
use std::path::Path;

#[derive(Debug, Default)]
pub struct SaveOptions {
    pub message: Option<String>,
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
    let scanned_files = scan_workspace(workspace_root, None)?;

    // 5. Open/create FileIndex
    let index = FileIndex::open(layout.index_path())?;
    let cached_entries = index.entries_by_path()?;

    // 6. Create blob store
    let blob_store = BlobStore::new(layout.objects_dir());
    let packs_dir = layout.packs_dir();
    let pack_set = PackSet::open_all(&packs_dir)?;
    let mut pack_writer = PackWriter::new();
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
    for prepared in prepare_files(files_to_prepare)? {
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
        if !staged_pack_hashes.contains(&blob_hash_hex)
            && !blob_store.exists(&blob_hash_hex)
            && !pack_set.contains(&blob_hash_hex)
        {
            pack_writer.add_pre_compressed(blob_hash_hex.clone(), compressed);
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
    if !staged_pack_hashes.is_empty() {
        pack_writer.finish(&packs_dir)?;
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

fn prepare_files(scanned_files: Vec<&ScannedFile>) -> Result<Vec<PreparedFile>> {
    if scanned_files.is_empty() {
        return Ok(Vec::new());
    }

    let worker_count = std::thread::available_parallelism()
        .map(|count| count.get())
        .unwrap_or(1)
        .min(scanned_files.len());
    if worker_count <= 1 {
        return scanned_files.into_iter().map(prepare_file).collect();
    }

    let chunk_size = scanned_files.len().div_ceil(worker_count);
    std::thread::scope(|scope| {
        let mut workers = Vec::new();
        for chunk in scanned_files.chunks(chunk_size) {
            workers.push(scope.spawn(move || -> Result<Vec<PreparedFile>> {
                chunk.iter().map(|scanned| prepare_file(scanned)).collect()
            }));
        }

        let mut prepared = Vec::new();
        for worker in workers {
            let chunk = worker
                .join()
                .map_err(|_| ChkpttError::Other("save worker thread panicked".into()))??;
            prepared.extend(chunk);
        }
        Ok(prepared)
    })
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
}

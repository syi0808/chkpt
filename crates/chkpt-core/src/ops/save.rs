use crate::config::{project_id_from_path, StoreLayout};
use crate::error::{ChkpttError, Result};
use crate::index::{FileEntry, FileIndex};
use crate::ops::lock::ProjectLock;
use crate::scanner::{scan_workspace, ScannedFile};
use crate::store::blob::{hash_content, BlobStore};
use crate::store::snapshot::{Snapshot, SnapshotAttachments, SnapshotStats, SnapshotStore};
use crate::store::tree::{EntryType, TreeEntry, TreeStore};
use std::collections::BTreeMap;
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
    mtime_secs: i64,
    mtime_nanos: i64,
    inode: Option<u64>,
    is_new_object: bool,
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

    // 7. Process each scanned file: check index, hash, store blob
    let mut processed_files = Vec::with_capacity(scanned_files.len());
    let mut new_objects: u64 = 0;
    let mut total_bytes: u64 = 0;

    for scanned in &scanned_files {
        let pf = process_file(
            scanned,
            cached_entries.get(&scanned.relative_path),
            &blob_store,
        )?;
        total_bytes += pf.size;
        if pf.is_new_object {
            new_objects += 1;
        }
        processed_files.push(pf);
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

    // 12. Update FileIndex with all current file entries
    let file_entries: Vec<FileEntry> = processed_files
        .iter()
        .map(|pf| FileEntry {
            path: pf.relative_path.clone(),
            blob_hash: pf.blob_hash_bytes,
            size: pf.size,
            mtime_secs: pf.mtime_secs,
            mtime_nanos: pf.mtime_nanos,
            inode: pf.inode,
            mode: pf.mode,
        })
        .collect();
    index.bulk_upsert(&file_entries)?;

    // 13. Lock released automatically via drop

    // 14. Return SaveResult
    Ok(SaveResult { snapshot_id, stats })
}

/// Process a single scanned file: check the index cache, hash, and store blob.
fn process_file(
    scanned: &ScannedFile,
    cached: Option<&FileEntry>,
    blob_store: &BlobStore,
) -> Result<ProcessedFile> {
    // Check index for cached entry
    if let Some(cached) = cached {
        // If mtime + size + inode match, skip re-hash
        if cached.mtime_secs == scanned.mtime_secs
            && cached.mtime_nanos == scanned.mtime_nanos
            && cached.size == scanned.size
            && cached.inode == scanned.inode
        {
            // Use cached hash - no new object
            return Ok(ProcessedFile {
                relative_path: scanned.relative_path.clone(),
                blob_hash_bytes: cached.blob_hash,
                size: scanned.size,
                mode: scanned.mode,
                mtime_secs: scanned.mtime_secs,
                mtime_nanos: scanned.mtime_nanos,
                inode: scanned.inode,
                is_new_object: false,
            });
        }
    }

    // Need to read, hash, and store
    let content = std::fs::read(&scanned.absolute_path)?;
    let blob_hash_hex = hash_content(&content);
    let blob_hash_bytes = hex_to_bytes(&blob_hash_hex)?;

    // Check if blob already exists (dedup across files)
    let is_new_object = blob_store.write_if_missing(&blob_hash_hex, &content)?;

    Ok(ProcessedFile {
        relative_path: scanned.relative_path.clone(),
        blob_hash_bytes,
        size: scanned.size,
        mode: scanned.mode,
        mtime_secs: scanned.mtime_secs,
        mtime_nanos: scanned.mtime_nanos,
        inode: scanned.inode,
        is_new_object,
    })
}

/// Build tree structure bottom-up from processed files.
///
/// Groups files by their directory path, creates tree entries for each file,
/// recursively builds subtrees for subdirectories, and returns the root tree hash.
fn build_tree(processed_files: &[ProcessedFile], tree_store: &TreeStore) -> Result<String> {
    // Group files by parent directory
    // Key: directory path (empty string for root), Value: list of (filename, hash, size, mode)
    let mut dir_files: BTreeMap<String, Vec<&ProcessedFile>> = BTreeMap::new();

    for pf in processed_files {
        let parent = if let Some(pos) = pf.relative_path.rfind('/') {
            pf.relative_path[..pos].to_string()
        } else {
            String::new() // root directory
        };
        dir_files.entry(parent).or_default().push(pf);
    }

    // Collect all unique directory paths (including intermediate ones)
    let mut all_dirs: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    all_dirs.insert(String::new()); // root always exists
    for dir in dir_files.keys() {
        if !dir.is_empty() {
            // Add this directory and all ancestor directories
            let parts: Vec<&str> = dir.split('/').collect();
            for i in 1..=parts.len() {
                all_dirs.insert(parts[..i].join("/"));
            }
        }
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
        for (sub_dir, sub_hash) in &dir_hashes {
            let parent_of_sub = if let Some(pos) = sub_dir.rfind('/') {
                &sub_dir[..pos]
            } else {
                "" // parent is root
            };
            if parent_of_sub == dir.as_str() {
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

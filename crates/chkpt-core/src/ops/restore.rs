use crate::config::{project_id_from_path, StoreLayout};
use crate::error::{ChkpttError, Result};
use crate::index::FileIndex;
use crate::ops::lock::ProjectLock;
use crate::store::blob::{hash_content, BlobStore};
use crate::store::pack::read_object;
use crate::store::snapshot::SnapshotStore;
use crate::store::tree::{EntryType, TreeStore};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

#[derive(Debug, Default)]
pub struct RestoreOptions {
    pub dry_run: bool,
}

#[derive(Debug)]
pub struct RestoreResult {
    pub snapshot_id: String,
    pub files_added: u64,
    pub files_changed: u64,
    pub files_removed: u64,
    pub files_unchanged: u64,
}

/// Convert a [u8; 32] to a 64-char hex string.
fn bytes_to_hex(bytes: &[u8; 32]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

/// Recursively walk a tree and collect all file entries as (relative_path, blob_hash_hex).
fn collect_tree_files(
    tree_store: &TreeStore,
    tree_hash_hex: &str,
    prefix: &str,
    result: &mut BTreeMap<String, String>,
) -> Result<()> {
    let entries = tree_store.read(tree_hash_hex)?;
    for entry in &entries {
        let path = if prefix.is_empty() {
            entry.name.clone()
        } else {
            format!("{}/{}", prefix, entry.name)
        };
        match entry.entry_type {
            EntryType::File => {
                let blob_hash_hex = bytes_to_hex(&entry.hash);
                result.insert(path, blob_hash_hex);
            }
            EntryType::Dir => {
                let subtree_hash_hex = bytes_to_hex(&entry.hash);
                collect_tree_files(tree_store, &subtree_hash_hex, &path, result)?;
            }
            EntryType::Symlink => {
                // Symlinks are not restored (consistent with scanner skipping them)
            }
        }
    }
    Ok(())
}

/// Scan the current workspace to get a mapping of (relative_path -> content_hash_hex).
///
/// This uses the scanner to discover files, then hashes each file to get the current
/// content hash for comparison with the target snapshot state.
fn scan_current_state(workspace_root: &Path) -> Result<BTreeMap<String, String>> {
    let scanned = crate::scanner::scan_workspace(workspace_root, None)?;
    let mut state = BTreeMap::new();
    for file in &scanned {
        let content = std::fs::read(&file.absolute_path)?;
        let hash = hash_content(&content);
        state.insert(file.relative_path.clone(), hash);
    }
    Ok(state)
}

/// Restore workspace to a snapshot state.
///
/// This is the main restore function that:
/// 1. Resolves the snapshot ID ("latest" or prefix match)
/// 2. Loads the snapshot and reconstructs the target file state from the tree
/// 3. Compares target state vs current workspace state
/// 4. Either reports what would change (dry_run) or performs the actual restore
pub fn restore(
    workspace_root: &Path,
    snapshot_id: &str,
    options: RestoreOptions,
) -> Result<RestoreResult> {
    // 1. Compute project_id, create StoreLayout
    let project_id = project_id_from_path(workspace_root);
    let layout = StoreLayout::new(&project_id);
    layout.ensure_dirs()?;

    // 2. Acquire project lock
    let _lock = ProjectLock::acquire(&layout.locks_dir())?;

    // 3. Resolve snapshot ID
    let snapshot_store = SnapshotStore::new(layout.snapshots_dir());
    let resolved_snapshot = if snapshot_id == "latest" {
        snapshot_store
            .latest()?
            .ok_or_else(|| ChkpttError::SnapshotNotFound("latest (no snapshots exist)".into()))?
    } else {
        // Try exact match first
        match snapshot_store.load(snapshot_id) {
            Ok(snap) => snap,
            Err(ChkpttError::SnapshotNotFound(_)) => {
                // Try prefix match
                let all_ids = snapshot_store.all_ids()?;
                let matches: Vec<_> = all_ids
                    .iter()
                    .filter(|id| id.starts_with(snapshot_id))
                    .collect();
                match matches.len() {
                    0 => {
                        return Err(ChkpttError::SnapshotNotFound(snapshot_id.to_string()));
                    }
                    1 => snapshot_store.load(matches[0])?,
                    _ => {
                        return Err(ChkpttError::Other(format!(
                            "Ambiguous snapshot prefix '{}': matches {} snapshots",
                            snapshot_id,
                            matches.len()
                        )));
                    }
                }
            }
            Err(e) => return Err(e),
        }
    };

    let resolved_id = resolved_snapshot.id.clone();

    // 4. Load snapshot's tree to get target state (path -> blob_hash_hex)
    let tree_store = TreeStore::new(layout.trees_dir());
    let root_tree_hash_hex = bytes_to_hex(&resolved_snapshot.root_tree_hash);
    let mut target_state: BTreeMap<String, String> = BTreeMap::new();
    collect_tree_files(&tree_store, &root_tree_hash_hex, "", &mut target_state)?;

    // 5. Scan current workspace to get current state (path -> content_hash_hex)
    let current_state = scan_current_state(workspace_root)?;

    // 6. Compare target state vs current state
    let target_paths: BTreeSet<&String> = target_state.keys().collect();
    let current_paths: BTreeSet<&String> = current_state.keys().collect();

    // Files to add: in target but not in current workspace
    let files_to_add: Vec<&String> = target_paths.difference(&current_paths).copied().collect();
    // Files to remove: in current workspace but not in target
    let files_to_remove: Vec<&String> = current_paths.difference(&target_paths).copied().collect();
    // Files in both: check if content differs
    let files_in_both: Vec<&String> = target_paths.intersection(&current_paths).copied().collect();

    let mut files_to_change: Vec<&String> = Vec::new();
    let mut files_unchanged: u64 = 0;

    for path in &files_in_both {
        let target_hash = &target_state[*path];
        let current_hash = &current_state[*path];
        if target_hash != current_hash {
            files_to_change.push(path);
        } else {
            files_unchanged += 1;
        }
    }

    let result = RestoreResult {
        snapshot_id: resolved_id.clone(),
        files_added: files_to_add.len() as u64,
        files_changed: files_to_change.len() as u64,
        files_removed: files_to_remove.len() as u64,
        files_unchanged,
    };

    // 7. If dry_run, return result without modifying workspace
    if options.dry_run {
        return Ok(result);
    }

    // 8. Perform actual restore
    let blob_store = BlobStore::new(layout.objects_dir());
    let packs_dir = layout.packs_dir();

    // 8a. Restore files that need to be added or changed
    for path in files_to_add.iter().chain(files_to_change.iter()) {
        let blob_hash_hex = &target_state[*path];
        let content = read_object(&blob_store, &packs_dir, blob_hash_hex)?;
        let file_path = workspace_root.join(path);

        // Create parent directories if they don't exist
        if let Some(parent) = file_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        std::fs::write(&file_path, &content)?;
    }

    // 8b. Remove files that are not in the target snapshot
    for path in &files_to_remove {
        let file_path = workspace_root.join(path);
        if file_path.exists() {
            std::fs::remove_file(&file_path)?;
        }
    }

    // 8c. Clean up empty directories
    cleanup_empty_dirs(workspace_root)?;

    // 9. Reset the FileIndex: clear and rebuild from target state
    let index = FileIndex::open(layout.index_path())?;
    index.clear()?;

    // Rebuild index from the restored workspace files
    let scanned = crate::scanner::scan_workspace(workspace_root, None)?;
    let file_entries: Vec<crate::index::FileEntry> = scanned
        .iter()
        .map(|sf| {
            let content = std::fs::read(&sf.absolute_path).unwrap_or_default();
            let hash_hex = hash_content(&content);
            let blob_hash = hex_to_bytes(&hash_hex);
            crate::index::FileEntry {
                path: sf.relative_path.clone(),
                blob_hash,
                size: sf.size,
                mtime_secs: sf.mtime_secs,
                mtime_nanos: sf.mtime_nanos,
                inode: sf.inode,
                mode: sf.mode,
            }
        })
        .collect();
    index.bulk_upsert(&file_entries)?;

    Ok(result)
}

/// Convert a 64-char hex string to a [u8; 32] array.
fn hex_to_bytes(hex: &str) -> [u8; 32] {
    let mut bytes = [0u8; 32];
    for i in 0..32 {
        if let Ok(b) = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16) {
            bytes[i] = b;
        }
    }
    bytes
}

/// Recursively remove empty directories under root (but not root itself).
fn cleanup_empty_dirs(root: &Path) -> Result<()> {
    cleanup_empty_dirs_recursive(root, root)
}

fn cleanup_empty_dirs_recursive(root: &Path, dir: &Path) -> Result<()> {
    if !dir.is_dir() {
        return Ok(());
    }

    // First recurse into subdirectories
    let entries: Vec<_> = std::fs::read_dir(dir)?
        .filter_map(|e| e.ok())
        .collect();

    for entry in &entries {
        if entry.file_type().map_or(false, |ft| ft.is_dir()) {
            cleanup_empty_dirs_recursive(root, &entry.path())?;
        }
    }

    // After recursing, check if directory is now empty (and it's not the root)
    if dir != root {
        let remaining: Vec<_> = std::fs::read_dir(dir)?
            .filter_map(|e| e.ok())
            .collect();
        if remaining.is_empty() {
            std::fs::remove_dir(dir)?;
        }
    }

    Ok(())
}

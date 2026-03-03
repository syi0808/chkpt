use crate::config::{project_id_from_path, StoreLayout};
use crate::error::Result;
use crate::ops::lock::ProjectLock;
use crate::store::blob::BlobStore;
use crate::store::snapshot::SnapshotStore;
use crate::store::tree::{EntryType, TreeStore};
use std::collections::HashSet;
use std::path::Path;

/// Convert a [u8; 32] to a 64-char hex string.
fn bytes_to_hex(bytes: &[u8; 32]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

/// Recursively walk a tree and collect all reachable blob and tree hashes.
///
/// This is used by the mark-and-sweep GC to determine which objects are still
/// referenced by remaining snapshots.
fn collect_reachable_hashes(
    tree_store: &TreeStore,
    tree_hash_hex: &str,
    reachable_blobs: &mut HashSet<String>,
    reachable_trees: &mut HashSet<String>,
) -> Result<()> {
    // Mark this tree as reachable
    if !reachable_trees.insert(tree_hash_hex.to_string()) {
        // Already visited this tree, skip to avoid redundant work
        return Ok(());
    }

    let entries = tree_store.read(tree_hash_hex)?;
    for entry in &entries {
        match entry.entry_type {
            EntryType::File => {
                let blob_hash_hex = bytes_to_hex(&entry.hash);
                reachable_blobs.insert(blob_hash_hex);
            }
            EntryType::Dir => {
                let subtree_hash_hex = bytes_to_hex(&entry.hash);
                collect_reachable_hashes(
                    tree_store,
                    &subtree_hash_hex,
                    reachable_blobs,
                    reachable_trees,
                )?;
            }
            EntryType::Symlink => {
                // Symlinks may store their target as a blob
                let blob_hash_hex = bytes_to_hex(&entry.hash);
                reachable_blobs.insert(blob_hash_hex);
            }
        }
    }
    Ok(())
}

/// Delete a snapshot and run mark-and-sweep garbage collection.
///
/// This function:
/// 1. Computes the project ID and store layout
/// 2. Acquires the project lock
/// 3. Verifies the snapshot exists, then deletes the snapshot JSON file
/// 4. Runs mark-and-sweep GC to remove unreachable loose objects:
///    a. Collects all reachable hashes from remaining snapshots
///    b. Lists all existing loose blob and tree hashes
///    c. Deletes unreachable loose blobs and trees
/// 5. Releases the lock (automatically via drop)
pub fn delete(workspace_root: &Path, snapshot_id: &str) -> Result<()> {
    // 1. Compute project_id, create StoreLayout
    let project_id = project_id_from_path(workspace_root);
    let layout = StoreLayout::new(&project_id);
    layout.ensure_dirs()?;

    // 2. Acquire project lock
    let _lock = ProjectLock::acquire(&layout.locks_dir())?;

    // 3. Verify snapshot exists, then delete it
    let snapshot_store = SnapshotStore::new(layout.snapshots_dir());
    // Load to verify it exists (will return SnapshotNotFound if not)
    snapshot_store.load(snapshot_id)?;
    snapshot_store.delete(snapshot_id)?;

    // 4. Run mark-and-sweep GC
    let tree_store = TreeStore::new(layout.trees_dir());
    let blob_store = BlobStore::new(layout.objects_dir());

    // 4a. Collect all reachable hashes from REMAINING snapshots
    let remaining_snapshots = snapshot_store.list(None)?;
    let mut reachable_blobs: HashSet<String> = HashSet::new();
    let mut reachable_trees: HashSet<String> = HashSet::new();

    for snapshot in &remaining_snapshots {
        let root_tree_hash_hex = bytes_to_hex(&snapshot.root_tree_hash);
        collect_reachable_hashes(
            &tree_store,
            &root_tree_hash_hex,
            &mut reachable_blobs,
            &mut reachable_trees,
        )?;
    }

    // 4b. Collect all existing loose blob hashes and tree hashes
    let all_loose_blobs = blob_store.list_loose()?;
    let all_loose_trees = tree_store.list_loose()?;

    // 4c. Delete unreachable loose blobs
    for blob_hash in &all_loose_blobs {
        if !reachable_blobs.contains(blob_hash) {
            blob_store.remove(blob_hash)?;
        }
    }

    // 4d. Delete unreachable loose trees
    for tree_hash in &all_loose_trees {
        if !reachable_trees.contains(tree_hash) {
            tree_store.remove(tree_hash)?;
        }
    }

    // 5. Lock released automatically via drop
    Ok(())
}

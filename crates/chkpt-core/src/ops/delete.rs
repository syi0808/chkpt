use crate::config::{project_id_from_path, StoreLayout};
use crate::error::{ChkpttError, Result};
use crate::ops::lock::ProjectLock;
use crate::store::blob::bytes_to_hex;
use crate::store::catalog::{CatalogSnapshot, MetadataCatalog};
use crate::store::pack::remove_pack_files;
use crate::store::tree::{EntryType, TreeStore};
use std::collections::HashSet;
use std::path::Path;

fn collect_reachable_blobs_from_tree(
    tree_store: &TreeStore,
    tree_hash: &[u8; 16],
    reachable_blobs: &mut HashSet<[u8; 16]>,
    visited: &mut HashSet<[u8; 16]>,
) -> Result<()> {
    if !visited.insert(*tree_hash) {
        return Ok(());
    }
    let entries = tree_store.read(&bytes_to_hex(tree_hash))?;
    for entry in entries {
        match entry.entry_type {
            EntryType::File | EntryType::Symlink => {
                reachable_blobs.insert(entry.hash);
            }
            EntryType::Dir => {
                collect_reachable_blobs_from_tree(
                    tree_store,
                    &entry.hash,
                    reachable_blobs,
                    visited,
                )?;
            }
        }
    }
    Ok(())
}

fn collect_reachable_blobs(
    catalog: &MetadataCatalog,
    tree_store: &TreeStore,
    snapshots: &[CatalogSnapshot],
) -> Result<Option<HashSet<[u8; 16]>>> {
    let mut reachable = HashSet::new();
    let mut visited: HashSet<[u8; 16]> = HashSet::new();

    for snapshot in snapshots {
        if snapshot.stats.total_files == 0 {
            continue;
        }

        let manifest = catalog.snapshot_manifest(&snapshot.id)?;
        if !manifest.is_empty() {
            reachable.extend(manifest.into_iter().map(|entry| entry.blob_hash));
            continue;
        }

        let Some(root_tree_hash) = snapshot.root_tree_hash else {
            return Ok(None);
        };
        if collect_reachable_blobs_from_tree(
            tree_store,
            &root_tree_hash,
            &mut reachable,
            &mut visited,
        )
        .is_err()
        {
            return Ok(None);
        }
    }

    Ok(Some(reachable))
}

pub fn delete(workspace_root: &Path, snapshot_id: &str) -> Result<()> {
    let project_id = project_id_from_path(workspace_root);
    let layout = StoreLayout::new(&project_id);
    layout.ensure_dirs()?;

    let _lock = ProjectLock::acquire(&layout.locks_dir())?;
    let catalog = MetadataCatalog::open(layout.catalog_path())?;
    catalog.load_snapshot(snapshot_id)?;
    catalog.delete_snapshot(snapshot_id)?;

    let mut touched_packs = HashSet::new();
    let remaining_snapshots = catalog.list_snapshots(None)?;
    let tree_store = TreeStore::new(layout.trees_dir());
    if let Some(reachable_blobs) =
        collect_reachable_blobs(&catalog, &tree_store, &remaining_snapshots)?
    {
        for blob_hash in catalog.all_blob_hashes()? {
            if reachable_blobs.contains(&blob_hash) {
                continue;
            }
            if let Some(location) = catalog.blob_location(&blob_hash)? {
                let pack_hash = location.pack_hash.clone().ok_or_else(|| {
                    ChkpttError::StoreCorrupted(format!(
                        "blob {} is not stored in a pack",
                        bytes_to_hex(&blob_hash)
                    ))
                })?;
                touched_packs.insert(pack_hash);
            }
            catalog.delete_blob_location(&blob_hash)?;
        }
    }

    for pack_hash in touched_packs {
        if catalog.pack_reference_count(&pack_hash)? == 0 {
            remove_pack_files(&layout.packs_dir(), &pack_hash)?;
        }
    }

    Ok(())
}

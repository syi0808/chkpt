use crate::config::{project_id_from_path, StoreLayout};
use crate::error::Result;
use crate::ops::lock::ProjectLock;
use crate::store::blob::BlobStore;
use crate::store::catalog::{CatalogSnapshot, MetadataCatalog};
use crate::store::snapshot::SnapshotStore;
use crate::store::tree::{EntryType, TreeStore};
use std::collections::HashSet;
use std::path::Path;

fn bytes_to_hex(bytes: &[u8; 32]) -> String {
    blake3::Hash::from(*bytes).to_hex().to_string()
}

fn collect_reachable_blobs_from_tree(
    tree_store: &TreeStore,
    tree_hash_hex: &str,
    reachable_blobs: &mut HashSet<[u8; 32]>,
) -> Result<()> {
    let entries = tree_store.read(tree_hash_hex)?;
    for entry in entries {
        match entry.entry_type {
            EntryType::File | EntryType::Symlink => {
                reachable_blobs.insert(entry.hash);
            }
            EntryType::Dir => {
                collect_reachable_blobs_from_tree(
                    tree_store,
                    &bytes_to_hex(&entry.hash),
                    reachable_blobs,
                )?;
            }
        }
    }
    Ok(())
}

fn collect_reachable_blobs(
    catalog: &MetadataCatalog,
    snapshot_store: &SnapshotStore,
    tree_store: &TreeStore,
    snapshots: &[CatalogSnapshot],
) -> Result<Option<HashSet<[u8; 32]>>> {
    let mut reachable = HashSet::new();

    for snapshot in snapshots {
        let manifest = catalog.snapshot_manifest(&snapshot.id)?;
        if !manifest.is_empty() {
            reachable.extend(manifest.into_iter().map(|entry| entry.blob_hash));
            continue;
        }

        let snapshot_json = match snapshot_store.load(&snapshot.id) {
            Ok(snapshot_json) => snapshot_json,
            Err(_) => return Ok(None),
        };
        if collect_reachable_blobs_from_tree(
            tree_store,
            &bytes_to_hex(&snapshot_json.root_tree_hash),
            &mut reachable,
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
    let snapshot_store = SnapshotStore::new(layout.snapshots_dir());
    snapshot_store.delete(snapshot_id)?;

    let blob_store = BlobStore::new(layout.objects_dir());
    let mut touched_packs = HashSet::new();
    let remaining_snapshots = catalog.list_snapshots(None)?;
    let tree_store = TreeStore::new(layout.trees_dir());
    if let Some(reachable_blobs) =
        collect_reachable_blobs(&catalog, &snapshot_store, &tree_store, &remaining_snapshots)?
    {
        for blob_hash in catalog.all_blob_hashes()? {
            if reachable_blobs.contains(&blob_hash) {
                continue;
            }
            if let Some(location) = catalog.blob_location(&blob_hash)? {
                if let Some(pack_hash) = location.pack_hash.clone() {
                    touched_packs.insert(pack_hash);
                } else {
                    blob_store.remove(&bytes_to_hex(&blob_hash))?;
                }
            }
            catalog.delete_blob_location(&blob_hash)?;
        }
    }

    for pack_hash in touched_packs {
        if catalog.pack_reference_count(&pack_hash)? == 0 {
            let dat_path = layout.packs_dir().join(format!("pack-{}.dat", pack_hash));
            let idx_path = layout.packs_dir().join(format!("pack-{}.idx", pack_hash));
            match std::fs::remove_file(dat_path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
            match std::fs::remove_file(idx_path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        }
    }

    Ok(())
}

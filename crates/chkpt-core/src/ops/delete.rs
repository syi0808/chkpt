use crate::config::{project_id_from_path, StoreLayout};
use crate::error::Result;
use crate::ops::lock::ProjectLock;
use crate::store::blob::BlobStore;
use crate::store::catalog::MetadataCatalog;
use crate::store::snapshot::SnapshotStore;
use std::collections::HashSet;
use std::path::Path;

fn bytes_to_hex(bytes: &[u8; 32]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

pub fn delete(workspace_root: &Path, snapshot_id: &str) -> Result<()> {
    let project_id = project_id_from_path(workspace_root);
    let layout = StoreLayout::new(&project_id);
    layout.ensure_dirs()?;

    let _lock = ProjectLock::acquire(&layout.locks_dir())?;

    let catalog = MetadataCatalog::open(layout.catalog_path())?;
    catalog.load_snapshot(snapshot_id)?;
    catalog.delete_snapshot(snapshot_id)?;
    SnapshotStore::new(layout.snapshots_dir()).delete(snapshot_id)?;

    let blob_store = BlobStore::new(layout.objects_dir());
    let mut touched_packs = HashSet::new();
    for (blob_hash, location) in catalog.unreferenced_blobs()? {
        if let Some(pack_hash) = location.pack_hash.clone() {
            touched_packs.insert(pack_hash);
        } else {
            blob_store.remove(&bytes_to_hex(&blob_hash))?;
        }
        catalog.delete_blob_location(&blob_hash)?;
    }

    for pack_hash in touched_packs {
        if catalog.pack_reference_count(&pack_hash)? == 0 {
            let dat_path = layout.packs_dir().join(format!("pack-{}.dat", pack_hash));
            let idx_path = layout.packs_dir().join(format!("pack-{}.idx", pack_hash));
            if dat_path.exists() {
                std::fs::remove_file(dat_path)?;
            }
            if idx_path.exists() {
                std::fs::remove_file(idx_path)?;
            }
        }
    }

    Ok(())
}

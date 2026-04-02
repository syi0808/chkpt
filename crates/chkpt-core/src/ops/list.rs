use crate::config::{project_id_from_path, StoreLayout};
use crate::error::Result;
use crate::store::catalog::{CatalogSnapshot, MetadataCatalog};
use crate::store::snapshot::Snapshot;
use std::path::Path;

pub fn list(workspace_root: &Path, limit: Option<usize>) -> Result<Vec<Snapshot>> {
    let project_id = project_id_from_path(workspace_root);
    let layout = StoreLayout::new(&project_id);
    let catalog = MetadataCatalog::open(layout.catalog_path())?;
    catalog
        .list_snapshots(limit)?
        .into_iter()
        .map(catalog_snapshot_to_public)
        .collect()
}

fn catalog_snapshot_to_public(snapshot: CatalogSnapshot) -> Result<Snapshot> {
    Ok(Snapshot {
        id: snapshot.id,
        created_at: snapshot.created_at,
        message: snapshot.message,
        root_tree_hash: [0u8; 32],
        parent_snapshot_id: snapshot.parent_snapshot_id,
        stats: snapshot.stats,
    })
}
